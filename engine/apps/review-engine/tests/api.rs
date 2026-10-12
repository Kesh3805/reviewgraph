#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end tests of the internal API against an in-memory backend: auth, tenant checks,
//! budgets, the auth-bypass path fixture, single-flight loading and excerpt redaction.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use codegraph::{
    Confidence, Edge, EdgeKind, Graph, GraphBuilder, NodeId, NodeInput, NodeKey, NodeKind,
    Provenance, ResolvedBy, SCHEMA_VERSION,
};
use http_body_util::BodyExt;
use impact::ImpactGraph;
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};
use review_core::location::RepoPath;
use review_engine::auth::{sign_token, unix_now, ServiceKeyRing};
use review_engine::backend::{BackendError, EngineBackend};
use review_engine::{router, AppState, EngineSettings};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const SECRET: &[u8] = b"engine-test-secret-0123456789abcdef";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn id_of(name: &str) -> String {
    format!("ts:src/{name}#{name}/function")
}

fn key_of(name: &str) -> NodeKey {
    NodeId::from_canonical(id_of(name)).key()
}

fn calls(from: &str, to: &str) -> Edge {
    Edge::new(
        EdgeKind::Calls,
        key_of(from),
        key_of(to),
        Confidence::from_f32(0.9),
        ResolvedBy::Import,
        Provenance::Linker,
    )
}

/// `update -> updateUser -> authorize` and `update -> audit -> authorize`, plus a hub with
/// forty leaves for the budget tests.
fn fixture() -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    let mut names = vec![
        "update".to_owned(),
        "updateUser".to_owned(),
        "authorize".to_owned(),
        "audit".to_owned(),
        "hub".to_owned(),
    ];
    names.extend((0..40).map(|i| format!("leaf{i}")));
    for name in &names {
        let node = NodeInput::new(
            NodeId::from_canonical(id_of(name)),
            NodeKind::Function,
            name,
        )
        .in_file(RepoPath::new(format!("src/{name}.ts")).unwrap());
        builder.add_node(node).unwrap();
    }
    builder.add_edge(calls("update", "updateUser"));
    builder.add_edge(calls("updateUser", "authorize"));
    builder.add_edge(calls("update", "audit"));
    builder.add_edge(calls("audit", "authorize"));
    for i in 0..40 {
        builder.add_edge(calls("hub", &format!("leaf{i}")));
    }
    builder.build().unwrap()
}

#[derive(Debug)]
struct MemBackend {
    graph: Arc<Graph>,
    scope: RepoScope,
    snapshot: SnapshotId,
    loads: AtomicUsize,
    delay: Duration,
    source: String,
}

impl MemBackend {
    fn check(&self, scope: RepoScope, snapshot: SnapshotId) -> Result<(), BackendError> {
        if scope == self.scope && snapshot == self.snapshot {
            Ok(())
        } else {
            Err(BackendError::NotFound)
        }
    }
}

#[async_trait]
impl EngineBackend for MemBackend {
    async fn load_graph(
        &self,
        scope: RepoScope,
        snapshot: SnapshotId,
    ) -> Result<Arc<Graph>, BackendError> {
        self.check(scope, snapshot)?;
        self.loads.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(Arc::clone(&self.graph))
    }

    async fn snapshot_commit(
        &self,
        scope: RepoScope,
        snapshot: SnapshotId,
    ) -> Result<String, BackendError> {
        self.check(scope, snapshot)?;
        Ok(COMMIT.to_owned())
    }

    async fn impact_graph(
        &self,
        _scope: RepoScope,
        _run: Uuid,
    ) -> Result<Option<ImpactGraph>, BackendError> {
        Ok(None)
    }

    async fn read_source(
        &self,
        scope: RepoScope,
        _commit_sha: &str,
        _path: &str,
    ) -> Result<String, BackendError> {
        if scope != self.scope {
            return Err(BackendError::NotFound);
        }
        Ok(self.source.clone())
    }
}

struct Harness {
    app: Router,
    backend: Arc<MemBackend>,
    org: Uuid,
    repo: Uuid,
    snap: Uuid,
}

fn harness(delay: Duration, load_wait: Duration, source: String) -> Harness {
    let (org, repo, snap) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let backend = Arc::new(MemBackend {
        graph: Arc::new(fixture()),
        scope: RepoScope {
            organization_id: OrganizationId::from_uuid(org),
            repository_id: RepositoryId::from_uuid(repo),
        },
        snapshot: SnapshotId::from_uuid(snap),
        loads: AtomicUsize::new(0),
        delay,
        source,
    });
    let state = AppState::new(
        ring(),
        backend.clone(),
        EngineSettings {
            graph_cache: 2,
            load_wait,
            query_parallelism: 4,
        },
    );
    Harness {
        app: router(state),
        backend,
        org,
        repo,
        snap,
    }
}

fn default_harness() -> Harness {
    harness(Duration::ZERO, Duration::from_secs(2), String::new())
}

fn ring() -> ServiceKeyRing {
    ServiceKeyRing::single("default", SECRET).unwrap()
}

fn token_with(claims: Value) -> String {
    sign_token(&ring(), &claims).unwrap()
}

fn claims(org: Uuid, repo: Uuid) -> Value {
    let now = unix_now();
    json!({
        "iss": "rg-api", "aud": "rg-engine", "sub": "api-test", "scope": ["graph:read"],
        "org": org.to_string(), "repo": repo.to_string(),
        "iat": now, "exp": now + 60, "jti": Uuid::new_v4().to_string(),
    })
}

fn token(h: &Harness) -> String {
    token_with(claims(h.org, h.repo))
}

struct Reply {
    status: StatusCode,
    retry_after: Option<String>,
    body: Value,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        retry_after,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn snap_uri(h: &Harness, rest: &str) -> String {
    format!("/internal/v1/repos/{}/snapshots/{}/{rest}", h.repo, h.snap)
}

#[tokio::test]
async fn routes_require_service_auth() {
    let h = default_harness();
    let uri = snap_uri(&h, "symbols?q=update");

    let health = send(&h.app, "GET", "/internal/v1/health", None, None).await;
    assert_eq!(health.status, StatusCode::OK);

    let routes = [
        ("GET", uri.clone()),
        (
            "GET",
            snap_uri(&h, &format!("symbols/{}", key_of("update"))),
        ),
        (
            "GET",
            snap_uri(&h, &format!("symbols/{}/neighbors", key_of("update"))),
        ),
        ("POST", snap_uri(&h, "subgraph")),
        ("GET", snap_uri(&h, "path?from=a&to=b")),
        ("GET", snap_uri(&h, "source?path=a.ts&start=1&end=2")),
        (
            "GET",
            format!("/internal/v1/reviews/{}/impact/x", Uuid::new_v4()),
        ),
    ];
    for (method, route) in &routes {
        let reply = send(&h.app, method, route, None, None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {route}");
        let reply = send(&h.app, method, route, Some("not.a.token"), None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {route}");
    }

    let mut wrong_audience = claims(h.org, h.repo);
    wrong_audience["aud"] = json!("rg-api");
    let reply = send(&h.app, "GET", &uri, Some(&token_with(wrong_audience)), None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    let mut wrong_scope = claims(h.org, h.repo);
    wrong_scope["scope"] = json!(["clone-credentials"]);
    let reply = send(&h.app, "GET", &uri, Some(&token_with(wrong_scope)), None).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    let good = token(&h);
    let reply = send(&h.app, "GET", &uri, Some(&good), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    // The same token (same jti) is a replay.
    let replay = send(&h.app, "GET", &uri, Some(&good), None).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn repo_claim_mismatch_403() {
    let h = default_harness();
    let other_repo = token_with(claims(h.org, Uuid::new_v4()));
    let reply = send(
        &h.app,
        "GET",
        &snap_uri(&h, "symbols?q=update"),
        Some(&other_repo),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    let mut no_repo = claims(h.org, h.repo);
    no_repo.as_object_mut().unwrap().remove("repo");
    let reply = send(
        &h.app,
        "GET",
        &snap_uri(&h, "symbols?q=update"),
        Some(&token_with(no_repo)),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(h.backend.loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn foreign_and_unknown_snapshots_are_404() {
    let h = default_harness();
    // Right repository claim, but a snapshot the backend does not know in this scope.
    let uri = format!(
        "/internal/v1/repos/{}/snapshots/{}/symbols?q=update",
        h.repo,
        Uuid::new_v4()
    );
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    // Another tenant using the real ids is told the same.
    let foreign = token_with(claims(Uuid::new_v4(), h.repo));
    let reply = send(
        &h.app,
        "GET",
        &snap_uri(&h, "symbols?q=update"),
        Some(&foreign),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn symbol_search_ranks_exact_before_prefix_before_substring() {
    let h = default_harness();
    let reply = send(
        &h.app,
        "GET",
        &snap_uri(&h, "symbols?q=update&limit=10"),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let names: Vec<&str> = reply.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["update", "updateUser"]);
    assert_eq!(reply.body["items"][0]["out_degree"], 2);

    let too_many = send(
        &h.app,
        "GET",
        &snap_uri(&h, "symbols?limit=51"),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(too_many.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn symbol_detail_and_neighbors() {
    let h = default_harness();
    let key = key_of("update");
    let detail = send(
        &h.app,
        "GET",
        &snap_uri(&h, &format!("symbols/{key}")),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.body["name"], "update");
    assert_eq!(detail.body["edges_out"]["CALLS"], 2);

    let neighbors = send(
        &h.app,
        "GET",
        &snap_uri(&h, &format!("symbols/{key}/neighbors?dir=out&kinds=calls")),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(neighbors.status, StatusCode::OK);
    assert_eq!(neighbors.body["edges"].as_array().unwrap().len(), 2);
    assert_eq!(neighbors.body["nodes"].as_array().unwrap().len(), 3);

    let bad_kind = send(
        &h.app,
        "GET",
        &snap_uri(&h, &format!("symbols/{key}/neighbors?kinds=NOPE")),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(bad_kind.status, StatusCode::BAD_REQUEST);

    let missing = send(
        &h.app,
        "GET",
        &snap_uri(&h, &format!("symbols/{}", "0".repeat(32))),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn subgraph_budget_truncates() {
    let h = default_harness();
    let hub = key_of("hub").to_string();
    let reply = send(
        &h.app,
        "POST",
        &snap_uri(&h, "subgraph"),
        Some(&token(&h)),
        Some(json!({ "seeds": [hub], "depth": 1, "max_nodes": 10 })),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["truncated"], true);
    assert!(reply.body["nodes"].as_array().unwrap().len() <= 10);

    let full = send(
        &h.app,
        "POST",
        &snap_uri(&h, "subgraph"),
        Some(&token(&h)),
        Some(json!({ "seeds": [hub], "depth": 1, "max_nodes": 100 })),
    )
    .await;
    assert_eq!(full.body["truncated"], false);
    assert_eq!(full.body["nodes"].as_array().unwrap().len(), 41);

    for body in [
        json!({ "seeds": [hub], "max_nodes": 501 }),
        json!({ "seeds": [hub], "depth": 4 }),
        json!({ "seeds": [] }),
    ] {
        let reply = send(
            &h.app,
            "POST",
            &snap_uri(&h, "subgraph"),
            Some(&token(&h)),
            Some(body),
        )
        .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn path_finds_auth_bypass_chain() {
    let h = default_harness();
    let from = key_of("update");
    let to = key_of("authorize");
    let uri = snap_uri(&h, &format!("path?from={from}&to={to}&max_depth=4"));
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let paths = reply.body["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 1);
    let names: Vec<&str> = paths[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.first(), Some(&"update"));
    assert_eq!(names.last(), Some(&"authorize"));
    assert_eq!(names.len(), 3);
    assert_eq!(paths[0]["edges"].as_array().unwrap().len(), 2);
    assert_eq!(paths[0]["edges"][0]["kind"], "CALLS");

    // `all=true` lists both two-hop chains.
    let uri = snap_uri(
        &h,
        &format!("path?from={from}&to={to}&all=true&max_paths=5&kinds=CALLS"),
    );
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.body["paths"].as_array().unwrap().len(), 2);
    assert_eq!(reply.body["truncated"], false);

    // One hop is not enough.
    let uri = snap_uri(
        &h,
        &format!("path?from={from}&to={to}&max_depth=1&all=true"),
    );
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.body["paths"].as_array().unwrap().len(), 0);

    // Edges point one way only.
    let uri = snap_uri(&h, &format!("path?from={to}&to={from}"));
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.body["paths"].as_array().unwrap().len(), 0);

    let uri = snap_uri(&h, &format!("path?from={from}&to={to}&max_depth=7"));
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn single_flight_load() {
    let h = harness(
        Duration::from_millis(150),
        Duration::from_secs(5),
        String::new(),
    );
    let uri = snap_uri(&h, "symbols?q=update");
    let calls = (0..8).map(|_| {
        let app = h.app.clone();
        let uri = uri.clone();
        let token = token(&h);
        async move { send(&app, "GET", &uri, Some(&token), None).await.status }
    });
    let statuses = futures::future::join_all(calls).await;
    assert!(statuses.iter().all(|s| *s == StatusCode::OK));
    assert_eq!(h.backend.loads.load(Ordering::SeqCst), 1);

    // Warm: no further load.
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(h.backend.loads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn slow_load_answers_503_and_keeps_loading() {
    let h = harness(
        Duration::from_millis(400),
        Duration::from_millis(50),
        String::new(),
    );
    let uri = snap_uri(&h, "symbols?q=update");
    let first = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(first.retry_after.as_deref(), Some("2"));

    tokio::time::sleep(Duration::from_millis(600)).await;
    let second = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(h.backend.loads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn excerpt_redacted_and_capped() {
    let mut source: Vec<String> = (1..=300).map(|n| format!("const line{n} = {n};")).collect();
    source[1] = "const key = \"AKIAIOSFODNN7EXAMPLE\";".to_owned();
    let h = harness(Duration::ZERO, Duration::from_secs(2), source.join("\n"));

    let reply = send(
        &h.app,
        "GET",
        &snap_uri(&h, "source?path=src/a.ts&start=1&end=250"),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let text = reply.body["text"].as_str().unwrap();
    assert!(!text.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(text.contains("redacted:aws_access_key_id"));
    assert_eq!(reply.body["redacted"], true);
    assert_eq!(reply.body["truncated"], true);
    assert_eq!(reply.body["end"], 200);
    assert_eq!(text.lines().count(), 200);

    let traversal = send(
        &h.app,
        "GET",
        &snap_uri(&h, "source?path=../etc/passwd&start=1&end=2"),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(traversal.status, StatusCode::BAD_REQUEST);
    let inverted = send(
        &h.app,
        "GET",
        &snap_uri(&h, "source?path=a.ts&start=5&end=2"),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(inverted.status, StatusCode::BAD_REQUEST);
    let past_end = send(
        &h.app,
        "GET",
        &snap_uri(&h, "source?path=a.ts&start=900&end=910"),
        Some(&token(&h)),
        None,
    )
    .await;
    assert_eq!(past_end.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn impact_of_an_unknown_run_is_404() {
    let h = default_harness();
    let uri = format!(
        "/internal/v1/reviews/{}/impact/{}",
        Uuid::new_v4(),
        key_of("update")
    );
    let reply = send(&h.app, "GET", &uri, Some(&token(&h)), None).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}
