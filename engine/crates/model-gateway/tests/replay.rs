#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use model_gateway::adapters::record::{RecordConfig, RecordingAdapter, FIXTURE_ORG_ID};
use model_gateway::adapters::replay::{ReplayAdapter, ReplayConfig};
use model_gateway::fixture::{Fixture, FixtureResponse, FixtureStore, ANY, FIXTURE_VERSION};
use model_gateway::{
    request_hash, FinishReason, GatewayBuilder, GatewayError, ModelGateway, ModelOutput,
    ModelRequest, ModelTier, PermanentKind, ProviderAdapter, ProviderId, ProviderRequest,
    ProviderResponse, RouteCandidate, ServedFrom, StaticRouter, Usage,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use common::request;

fn candidate() -> RouteCandidate {
    RouteCandidate {
        provider: ProviderId::new("anthropic"),
        model: "claude-sonnet-5-5".into(),
        max_context: 200_000,
        supports_reasoning: false,
    }
}

fn fixture_for(req: &ModelRequest, provider: &str, model: &str, output: Value) -> Fixture {
    Fixture {
        fixture_version: FIXTURE_VERSION,
        request_hash: request_hash(req).0,
        provider: provider.into(),
        model: model.into(),
        task: req.task.as_str().into(),
        prompt_id: req.input.system.prompt_id.clone(),
        prompt_version: req.input.system.prompt_version.clone(),
        schema_hash: req.output_schema.as_ref().map(|s| s.hash.clone()),
        synthetic: provider == ANY,
        recorded_at: "2026-01-01T00:00:00Z".into(),
        engine_git_sha: None,
        response: FixtureResponse {
            output: ModelOutput::Json(output),
            usage: Usage {
                input_uncached: 10,
                cache_write: 0,
                cache_read: 5,
                output: 7,
                reasoning: 0,
            },
            finish_reason: FinishReason::Complete,
        },
        latency_ms: 0,
    }
}

fn replay_gateway(dir: &tempfile::TempDir) -> model_gateway::Gateway {
    let store = Arc::new(FixtureStore::new(dir.path().join("fixtures")));
    let mut cfg = ReplayConfig::new(dir.path().join("fixtures"));
    cfg.miss_log = Some(dir.path().join("misses.jsonl"));
    let router = StaticRouter::new().with_route(ModelTier::ReviewReasoner, vec![candidate()]);
    GatewayBuilder::new()
        .adapter(Arc::new(ReplayAdapter::new(
            ProviderId::new("anthropic"),
            store,
            cfg,
        )))
        .router(Arc::new(router))
        .build()
        .expect("build")
}

#[tokio::test]
async fn replay_exact_match_served() {
    let dir = tempfile::tempdir().expect("tmp");
    let req = request();
    let store = FixtureStore::new(dir.path().join("fixtures"));
    store
        .write(
            req.task,
            &fixture_for(&req, "anthropic", "claude-sonnet-5-5", json!({"ok": true})),
            false,
        )
        .await
        .expect("write");
    let resp = replay_gateway(&dir)
        .call(req, CancellationToken::new())
        .await
        .expect("served");
    assert_eq!(resp.served_from, ServedFrom::Replay);
    assert_eq!(resp.output, ModelOutput::Json(json!({"ok": true})));
    assert_eq!(resp.usage.cache_read, 5);
}

#[tokio::test]
async fn replay_any_any_fallback_for_synthetic() {
    let dir = tempfile::tempdir().expect("tmp");
    let req = request();
    let store = FixtureStore::new(dir.path().join("fixtures"));
    store
        .write(
            req.task,
            &fixture_for(&req, ANY, ANY, json!({"ok": false})),
            false,
        )
        .await
        .expect("write");
    let resp = replay_gateway(&dir)
        .call(req, CancellationToken::new())
        .await
        .expect("served");
    assert_eq!(resp.output, ModelOutput::Json(json!({"ok": false})));
    assert_eq!(resp.model, "claude-sonnet-5-5");
}

#[tokio::test]
async fn replay_miss_is_permanent_and_logged() {
    let dir = tempfile::tempdir().expect("tmp");
    let err = replay_gateway(&dir)
        .call(request(), CancellationToken::new())
        .await
        .expect_err("miss");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::ReplayMiss,
            ..
        }
    ));
    assert!(!err.fallback_eligible());
    let log = std::fs::read_to_string(dir.path().join("misses.jsonl")).expect("miss log");
    let line: Value = serde_json::from_str(log.lines().next().expect("line")).expect("json");
    assert_eq!(line["request_hash"], request_hash(&request()).0);
    assert_eq!(line["task"], "correctness_review");
    assert_eq!(line["section_names"], json!(["rules", "diff"]));
}

#[tokio::test]
async fn corrupt_fixture_is_permanent_unknown() {
    let dir = tempfile::tempdir().expect("tmp");
    let req = request();
    let path = model_gateway::fixture::fixture_path(
        &dir.path().join("fixtures"),
        req.task,
        &request_hash(&req).0,
        "anthropic",
        "claude-sonnet-5-5",
    );
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, "{not json").expect("write");
    let err = replay_gateway(&dir)
        .call(req, CancellationToken::new())
        .await
        .expect_err("corrupt");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::Unknown,
            ..
        }
    ));
}

struct LiveAdapter {
    calls: Arc<AtomicUsize>,
    output: Value,
}

#[async_trait]
impl ProviderAdapter for LiveAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    async fn send(&self, _req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ProviderResponse {
            output: ModelOutput::Json(self.output.clone()),
            usage: Usage::default(),
            finish_reason: FinishReason::Complete,
            model: "claude-sonnet-5-5".into(),
            provider_request_id: None,
            tool_use_id: None,
            served_from: ServedFrom::Live,
        })
    }
}

fn recorder(
    dir: &tempfile::TempDir,
    output: Value,
    overwrite: bool,
) -> (model_gateway::Gateway, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(FixtureStore::new(dir.path().join("fixtures")));
    let rec = RecordingAdapter::new(
        Arc::new(LiveAdapter {
            calls: calls.clone(),
            output,
        }),
        store,
        RecordConfig {
            enabled: true,
            overwrite,
        },
    )
    .expect("recorder");
    let router = StaticRouter::new().with_route(ModelTier::ReviewReasoner, vec![candidate()]);
    let gw = GatewayBuilder::new()
        .adapter(Arc::new(rec))
        .router(Arc::new(router))
        .build()
        .expect("build");
    (gw, calls)
}

fn fixture_req() -> ModelRequest {
    let mut r = request();
    r.tenant.organization_id = FIXTURE_ORG_ID;
    r
}

fn fixture_files(dir: &tempfile::TempDir) -> Vec<PathBuf> {
    fn walk(p: &std::path::Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&dir.path().join("fixtures"), &mut out);
    out
}

#[tokio::test]
async fn record_mode_refuses_non_fixture_org() {
    let dir = tempfile::tempdir().expect("tmp");
    let (gw, calls) = recorder(&dir, json!({"ok": true}), false);
    let err = gw
        .call(request(), CancellationToken::new())
        .await
        .expect_err("refused");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::InvalidRequest,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(fixture_files(&dir).is_empty());
}

#[test]
fn record_mode_requires_explicit_flag() {
    let dir = tempfile::tempdir().expect("tmp");
    let calls = Arc::new(AtomicUsize::new(0));
    let res = RecordingAdapter::new(
        Arc::new(LiveAdapter {
            calls,
            output: json!({}),
        }),
        Arc::new(FixtureStore::new(dir.path())),
        RecordConfig::default(),
    );
    assert!(res.is_err());
    let cfg = RecordConfig::from_lookup(|k| (k == "MODEL_GATEWAY_RECORD").then(|| "1".to_owned()));
    assert!(cfg.enabled && !cfg.overwrite);
}

#[tokio::test]
async fn recorder_writes_atomically_and_never_overwrites() {
    let dir = tempfile::tempdir().expect("tmp");
    let (gw, _) = recorder(&dir, json!({"ok": true}), false);
    gw.call(fixture_req(), CancellationToken::new())
        .await
        .expect("recorded");
    let files = fixture_files(&dir);
    assert_eq!(files.len(), 1, "{files:?}");
    let first = std::fs::read_to_string(&files[0]).expect("read");

    // A second recorder with a different answer must not replace the fixture.
    let (gw2, _) = recorder(&dir, json!({"ok": false}), false);
    gw2.call(fixture_req(), CancellationToken::new())
        .await
        .expect("served live");
    assert_eq!(std::fs::read_to_string(&files[0]).expect("read"), first);

    // Explicit overwrite replaces it.
    let (gw3, _) = recorder(&dir, json!({"ok": false}), true);
    gw3.call(fixture_req(), CancellationToken::new())
        .await
        .expect("served live");
    assert_ne!(std::fs::read_to_string(&files[0]).expect("read"), first);
    assert_eq!(fixture_files(&dir).len(), 1, "no temp files left behind");

    // The recorded fixture replays.
    let replay: Fixture =
        serde_json::from_str(&std::fs::read_to_string(&files[0]).expect("read")).expect("fixture");
    assert_eq!(replay.request_hash, request_hash(&fixture_req()).0);
}

#[tokio::test]
async fn concurrent_recorders_produce_one_fixture() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = Arc::new(FixtureStore::new(dir.path().join("fixtures")));
    let req = fixture_req();
    let fx = fixture_for(&req, "anthropic", "m", json!({"ok": true}));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let s = store.clone();
        let f = fx.clone();
        let task = req.task;
        handles.push(tokio::spawn(async move { s.write(task, &f, false).await }));
    }
    let mut written = 0;
    for h in handles {
        if h.await.expect("join").expect("io") {
            written += 1;
        }
    }
    assert_eq!(written, 1);
    assert_eq!(fixture_files(&dir).len(), 1);
}

#[tokio::test]
async fn recorder_refuses_secret_in_output() {
    let dir = tempfile::tempdir().expect("tmp");
    let (gw, _) = recorder(
        &dir,
        json!({"ok": true, "note": "token ghp_abcdefghijklmnopqrstuvwxyz0123"}),
        false,
    );
    // The schema forbids extra properties, so use a permissive text instead.
    let mut req = fixture_req();
    req.output_schema = None;
    gw.call(req, CancellationToken::new())
        .await
        .expect("served live");
    assert!(fixture_files(&dir).is_empty());
}

#[tokio::test]
async fn recorder_skips_schema_invalid_output() {
    let dir = tempfile::tempdir().expect("tmp");
    let (gw, _) = recorder(&dir, json!({"ok": "not a bool"}), false);
    gw.call(fixture_req(), CancellationToken::new())
        .await
        .expect("served live");
    assert!(fixture_files(&dir).is_empty());
}

#[test]
fn fixture_schema_validates_all_committed_fixtures() {
    let schema_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/replay-fixture.v1.schema.json");
    let rendered = {
        let s = schemars::gen::SchemaGenerator::default().into_root_schema_for::<Fixture>();
        let mut t = serde_json::to_string_pretty(&serde_json::to_value(s).expect("schema"))
            .expect("render");
        t.push('\n');
        t
    };
    if std::env::var("UPDATE_SCHEMAS").is_ok() {
        std::fs::write(&schema_path, &rendered).expect("write schema");
    }
    let on_disk =
        std::fs::read_to_string(&schema_path).expect("schema file; run with UPDATE_SCHEMAS=1");
    assert_eq!(
        on_disk, rendered,
        "fixture schema drifted; run with UPDATE_SCHEMAS=1"
    );

    let schema: Value = serde_json::from_str(&on_disk).expect("json");
    let validator = jsonschema::validator_for(&schema).expect("compiles");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/model-replay");
    let mut stack = vec![root];
    let mut checked = 0;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "json") {
                let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).expect("read"))
                    .expect("json");
                assert!(
                    validator.is_valid(&v),
                    "{} does not match the fixture schema",
                    p.display()
                );
                checked += 1;
            }
        }
    }
    eprintln!("validated {checked} committed fixtures");
}
