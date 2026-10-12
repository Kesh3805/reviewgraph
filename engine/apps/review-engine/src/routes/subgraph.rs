//! `POST .../subgraph`: a bounded, induced neighborhood of the seeds.

use axum::extract::{Path, State};
use axum::Json;
use codegraph::{Direction, GraphQuery, SubgraphSpec, TraversalError};
use review_core::ids::SnapshotId;
use uuid::Uuid;

use super::{parse_key, parse_kinds, scope_of, Authed};
use crate::dto::{
    GraphEdgeDto, GraphNodeDto, SubgraphRequest, SubgraphResponse, MAX_SUBGRAPH_DEPTH,
    MAX_SUBGRAPH_NODES, MAX_SUBGRAPH_SEEDS,
};
use crate::error::ApiError;
use crate::state::AppState;

pub async fn subgraph(
    State(state): State<AppState>,
    claims: Authed,
    Path((repo, snap)): Path<(Uuid, Uuid)>,
    Json(request): Json<SubgraphRequest>,
) -> Result<Json<SubgraphResponse>, ApiError> {
    let scope = scope_of(&claims.0, repo)?;
    if request.seeds.is_empty() || request.seeds.len() > MAX_SUBGRAPH_SEEDS {
        return Err(ApiError::BadRequest(format!(
            "seeds must hold 1..={MAX_SUBGRAPH_SEEDS} entries"
        )));
    }
    let depth = request.depth.unwrap_or(1);
    if depth == 0 || depth > MAX_SUBGRAPH_DEPTH {
        return Err(ApiError::BadRequest(format!(
            "depth must be 1..={MAX_SUBGRAPH_DEPTH}"
        )));
    }
    let max_nodes = request.max_nodes.unwrap_or(200);
    if max_nodes == 0 || max_nodes > MAX_SUBGRAPH_NODES {
        return Err(ApiError::BadRequest(format!(
            "max_nodes must be 1..={MAX_SUBGRAPH_NODES}"
        )));
    }
    let kinds = parse_kinds(
        request
            .kinds
            .iter()
            .flat_map(|kinds| kinds.iter().map(String::as_str)),
    )?;
    let seeds: Vec<codegraph::NodeKey> = request.seeds.iter().map(|raw| parse_key(raw)).collect();
    let graph = state.graph(scope, SnapshotId::from_uuid(snap)).await?;
    let snapshot_id = snap.to_string();
    let response = state
        .blocking("subgraph", move || {
            let mut spec = SubgraphSpec::new(seeds, depth, Direction::Both, kinds);
            spec.max_nodes = max_nodes;
            let query: &dyn GraphQuery = &*graph;
            let result = codegraph::subgraph(query, &spec).map_err(|error| match error {
                TraversalError::NoValidSeeds => ApiError::NotFound,
                other => ApiError::BadRequest(other.to_string()),
            })?;
            let nodes = result
                .nodes
                .iter()
                .map(|node| GraphNodeDto {
                    key: node.key.to_string(),
                    id: node.id.clone(),
                    name: node.name.clone(),
                    qualified_name: node.qualified_name.clone(),
                    kind: node.kind.as_str().to_owned(),
                    path: node.file.clone(),
                    line: node.range.map(|range| range.start.line),
                })
                .collect();
            let edges = result
                .edges
                .iter()
                .map(|edge| GraphEdgeDto {
                    source: edge.source.to_string(),
                    target: edge.target.to_string(),
                    kind: edge.kind.as_str().to_owned(),
                    confidence: edge.confidence.as_f32(),
                    resolved_by: edge.resolved_by.as_str().to_owned(),
                })
                .collect();
            Ok(SubgraphResponse {
                snapshot_id,
                nodes,
                edges,
                truncated: result.truncated,
            })
        })
        .await?;
    Ok(Json(response))
}
