//! `GET .../path?from&to&max_depth&kinds&all&max_paths`.
//!
//! Without `all` the answer is the single minimum-hop path (`codegraph::shortest_path`). With
//! `all=true` the engine returns up to `max_paths` simple paths, shortest first, found by
//! iterative deepening under a fixed expansion budget; running out of budget is reported as
//! `truncated`, never as "no path".

use std::collections::{BTreeMap, HashSet};
use std::ops::ControlFlow;

use axum::extract::{Path, Query, State};
use axum::Json;
use codegraph::{
    Confidence, Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, NodeKey, PathError,
    PathSpec, ResolvedBy,
};
use review_core::ids::SnapshotId;
use uuid::Uuid;

use super::{node_dto, parse_key, parse_kinds, scope_of, Authed};
use crate::dto::{
    GraphEdgeDto, GraphNodeDto, PathDto, PathQuery, PathResponse, DEFAULT_ALL_PATHS, MAX_PATHS,
    MAX_PATH_DEPTH,
};
use crate::error::ApiError;
use crate::state::AppState;

/// Node expansions the multi-path search may spend across all deepening rounds.
pub const MULTI_PATH_BUDGET: u32 = 20_000;

#[derive(Debug, Clone, Copy)]
struct Step {
    source: NodeKey,
    target: NodeKey,
    kind: EdgeKind,
    confidence: Confidence,
    resolved_by: ResolvedBy,
}

/// The best edge to every distinct out-neighbor of `at` (two kinds between the same pair of
/// nodes are one hop of the path), in key order.
fn out_steps(graph: &dyn GraphQuery, at: NodeKey, filter: &EdgeFilter) -> Vec<Step> {
    let mut best: BTreeMap<NodeKey, Step> = BTreeMap::new();
    graph.for_each_edge(at, Direction::Out, filter, &mut |edge| {
        let better = best
            .get(&edge.target)
            .is_none_or(|current| edge.confidence > current.confidence);
        if better {
            best.insert(
                edge.target,
                Step {
                    source: edge.source,
                    target: edge.target,
                    kind: edge.kind,
                    confidence: edge.confidence,
                    resolved_by: edge.resolved_by,
                },
            );
        }
        ControlFlow::Continue(())
    });
    best.into_values().collect()
}

struct Search<'a> {
    graph: &'a dyn GraphQuery,
    to: NodeKey,
    filter: EdgeFilter,
    max_paths: usize,
    expansions: u32,
    truncated: bool,
    found: Vec<Vec<Step>>,
}

impl Search<'_> {
    fn done(&self) -> bool {
        self.truncated || self.found.len() >= self.max_paths
    }

    /// Records every simple path of exactly `remaining` more hops from `at` to the target.
    fn dfs(
        &mut self,
        at: NodeKey,
        remaining: u8,
        visited: &mut HashSet<NodeKey>,
        trail: &mut Vec<Step>,
    ) {
        if self.done() {
            return;
        }
        self.expansions += 1;
        if self.expansions > MULTI_PATH_BUDGET {
            self.truncated = true;
            return;
        }
        for step in out_steps(self.graph, at, &self.filter) {
            if self.done() {
                return;
            }
            if visited.contains(&step.target) {
                continue;
            }
            if step.target == self.to {
                // A simple path ends at the target and never passes through it.
                if remaining == 1 {
                    trail.push(step);
                    self.found.push(trail.clone());
                    trail.pop();
                }
                continue;
            }
            if remaining > 1 {
                visited.insert(step.target);
                trail.push(step);
                self.dfs(step.target, remaining - 1, visited, trail);
                trail.pop();
                visited.remove(&step.target);
            }
        }
    }
}

fn path_dto(graph: &dyn GraphQuery, from: NodeKey, steps: &[Step]) -> PathDto {
    let mut keys = vec![from];
    keys.extend(steps.iter().map(|step| step.target));
    let nodes: Vec<GraphNodeDto> = keys
        .iter()
        .filter_map(|key| graph.node(*key))
        .map(|node| node_dto(&node))
        .collect();
    let edges = steps
        .iter()
        .map(|step| GraphEdgeDto {
            source: step.source.to_string(),
            target: step.target.to_string(),
            kind: step.kind.as_str().to_owned(),
            confidence: step.confidence.as_f32(),
            resolved_by: step.resolved_by.as_str().to_owned(),
        })
        .collect();
    let weakest = steps
        .iter()
        .map(|step| step.confidence)
        .min()
        .unwrap_or(Confidence::MAX);
    PathDto {
        nodes,
        edges,
        min_confidence: weakest.as_f32(),
    }
}

fn find_paths(
    graph: &dyn GraphQuery,
    from: NodeKey,
    to: NodeKey,
    kinds: EdgeKindSet,
    max_depth: u8,
    all: bool,
    max_paths: usize,
) -> Result<(Vec<Vec<Step>>, bool), ApiError> {
    if graph.node(from).is_none() || graph.node(to).is_none() {
        return Err(ApiError::NotFound);
    }
    if !all {
        let mut spec = PathSpec::new(from, to, Direction::Out, kinds);
        spec.max_depth = max_depth;
        let result = codegraph::shortest_path(graph, &spec).map_err(|error| match error {
            PathError::UnknownNode(_) => ApiError::NotFound,
            other => ApiError::BadRequest(other.to_string()),
        })?;
        let paths = result
            .path
            .map(|path| {
                vec![path
                    .steps
                    .iter()
                    .map(|step| Step {
                        source: step.source,
                        target: step.target,
                        kind: step.kind,
                        confidence: step.confidence,
                        resolved_by: step.resolved_by,
                    })
                    .collect::<Vec<Step>>()]
            })
            .unwrap_or_default();
        return Ok((paths, result.truncated));
    }
    if from == to {
        return Ok((vec![Vec::new()], false));
    }
    let mut search = Search {
        graph,
        to,
        filter: EdgeFilter::kinds(kinds),
        max_paths,
        expansions: 0,
        truncated: false,
        found: Vec::new(),
    };
    for depth in 1..=max_depth {
        if search.done() {
            break;
        }
        let mut visited = HashSet::from([from]);
        let mut trail = Vec::new();
        search.dfs(from, depth, &mut visited, &mut trail);
    }
    Ok((search.found, search.truncated))
}

pub async fn path(
    State(state): State<AppState>,
    claims: Authed,
    Path((repo, snap)): Path<(Uuid, Uuid)>,
    Query(query): Query<PathQuery>,
) -> Result<Json<PathResponse>, ApiError> {
    let scope = scope_of(&claims.0, repo)?;
    let max_depth = query.max_depth.unwrap_or(4);
    if max_depth == 0 || max_depth > MAX_PATH_DEPTH {
        return Err(ApiError::BadRequest(format!(
            "max_depth must be 1..={MAX_PATH_DEPTH}"
        )));
    }
    let all = query.all.unwrap_or(false);
    let max_paths = query
        .max_paths
        .unwrap_or(if all { DEFAULT_ALL_PATHS } else { 1 });
    if max_paths == 0 || max_paths > MAX_PATHS {
        return Err(ApiError::BadRequest(format!(
            "max_paths must be 1..={MAX_PATHS}"
        )));
    }
    let kinds = match query.kinds.as_deref() {
        None => EdgeKindSet::ALL,
        Some(raw) => parse_kinds(raw.split(','))?,
    };
    let from = parse_key(&query.from);
    let to = parse_key(&query.to);
    let graph = state.graph(scope, SnapshotId::from_uuid(snap)).await?;
    let snapshot_id = snap.to_string();
    let response = state
        .blocking("path", move || {
            let query: &dyn GraphQuery = &*graph;
            let (paths, truncated) =
                find_paths(query, from, to, kinds, max_depth, all, max_paths as usize)?;
            Ok(PathResponse {
                snapshot_id,
                paths: paths
                    .iter()
                    .map(|steps| path_dto(query, from, steps))
                    .collect(),
                truncated,
            })
        })
        .await?;
    Ok(Json(response))
}
