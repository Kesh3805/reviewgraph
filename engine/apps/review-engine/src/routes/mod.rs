//! The `/internal/v1` router and the helpers its handlers share.
//!
//! Every route except `health` sits behind ServiceAuth. A handler then (1) compares the token's
//! `repo` claim with the repository in the path, (2) loads the snapshot through the scoped
//! backend (a snapshot of another repository or tenant is a 404), and (3) runs the traversal on
//! the blocking pool under an explicit budget.

pub mod impact;
pub mod path;
pub mod source;
pub mod subgraph;
pub mod symbols;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::{Json, Router};
use codegraph::{EdgeKind, EdgeKindSet, EdgeRef, NodeKey, NodeRef};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId};
use uuid::Uuid;

use crate::auth::{require_service_auth, Claims};
use crate::dto::{GraphEdgeDto, GraphNodeDto};
use crate::error::ApiError;
use crate::state::AppState;

/// Builds the full router. `state` carries the key ring, the backend and the caches.
pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route(
            "/repos/{repo}/snapshots/{snap}/symbols",
            get(symbols::search),
        )
        .route(
            "/repos/{repo}/snapshots/{snap}/symbols/{key}",
            get(symbols::detail),
        )
        .route(
            "/repos/{repo}/snapshots/{snap}/symbols/{key}/neighbors",
            get(symbols::neighbors),
        )
        .route(
            "/repos/{repo}/snapshots/{snap}/subgraph",
            post(subgraph::subgraph),
        )
        .route("/repos/{repo}/snapshots/{snap}/path", get(path::path))
        .route("/repos/{repo}/snapshots/{snap}/source", get(source::source))
        .route("/reviews/{run}/impact/{key}", get(impact::impact))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_service_auth,
        ));
    Router::new()
        // Orchestrator probes cannot mint service tokens, so liveness is the one open route.
        .route("/internal/v1/health", get(health))
        .nest("/internal/v1", protected)
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "review-engine" }))
}

/// The tenant scope of a request: the token's organization, and the path repository once it has
/// been checked against the token's `repo` claim.
pub(crate) fn scope_of(claims: &Claims, repo: Uuid) -> Result<RepoScope, ApiError> {
    let (org, repo) = claims.authorize_repo(repo)?;
    Ok(RepoScope {
        organization_id: OrganizationId::from_uuid(org),
        repository_id: RepositoryId::from_uuid(repo),
    })
}

pub(crate) type Authed = axum::Extension<Arc<Claims>>;

/// A node reference on the wire: the 32-character hex key, or a canonical node id.
pub(crate) fn parse_key(raw: &str) -> NodeKey {
    match raw.parse::<NodeKey>() {
        Ok(key) => key,
        Err(_) => codegraph::NodeId::from_canonical(raw).key(),
    }
}

/// `CALLS,IMPORTS` (case-insensitive) into a kind set; empty means every kind.
pub(crate) fn parse_kinds<'a>(
    raw: impl IntoIterator<Item = &'a str>,
) -> Result<EdgeKindSet, ApiError> {
    let mut set = EdgeKindSet::EMPTY;
    let mut any = false;
    for item in raw {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        any = true;
        let kind = EdgeKind::from_str_exact(&item.to_ascii_uppercase())
            .ok_or_else(|| ApiError::BadRequest(format!("unknown edge kind {item:?}")))?;
        set.insert(kind);
    }
    Ok(if any { set } else { EdgeKindSet::ALL })
}

pub(crate) fn node_dto(node: &NodeRef<'_>) -> GraphNodeDto {
    GraphNodeDto {
        key: node.key.to_string(),
        id: node.id.to_owned(),
        name: node.name.to_owned(),
        qualified_name: node.qualified_name.to_owned(),
        kind: node.kind.as_str().to_owned(),
        path: node.file.map(str::to_owned),
        line: node.range.map(|range| range.start.line),
    }
}

pub(crate) fn edge_dto(edge: &EdgeRef<'_>) -> GraphEdgeDto {
    GraphEdgeDto {
        source: edge.source.to_string(),
        target: edge.target.to_string(),
        kind: edge.kind.as_str().to_owned(),
        confidence: edge.confidence.as_f32(),
        resolved_by: edge.resolved_by.as_str().to_owned(),
    }
}

/// Counts of a per-kind map, for the symbol detail view.
pub(crate) fn bump(map: &mut BTreeMap<String, u32>, kind: &str) {
    let slot = map.entry(kind.to_owned()).or_insert(0);
    *slot = slot.saturating_add(1);
}
