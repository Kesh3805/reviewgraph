//! Symbol search, symbol detail and single-hop neighbors.

use std::collections::BTreeMap;
use std::ops::ControlFlow;

use axum::extract::{Path, Query, State};
use axum::Json;
use codegraph::{Confidence, Direction, EdgeFilter, EdgeKindSet, GraphQuery, NodeKey, NodeKind};
use review_core::ids::SnapshotId;
use uuid::Uuid;

use super::{bump, edge_dto, node_dto, parse_key, parse_kinds, scope_of, Authed};
use crate::dto::{
    FrameworkFact, NeighborsQuery, NeighborsResponse, SymbolDetail, SymbolSearchItem,
    SymbolSearchResponse, SymbolsQuery, MAX_NEIGHBOR_EDGES, MAX_SYMBOL_RESULTS,
};
use crate::error::ApiError;
use crate::state::AppState;

/// More matches than this are not ranked: the query is too broad and the answer is truncated.
const MAX_CANDIDATES: usize = 20_000;

/// Does a node kind satisfy the (case-insensitive) kind filter? The explorer's short names are
/// aliases: `endpoint`, `table` and `test` cover the family.
fn kind_matches(filter: &str, kind: NodeKind) -> bool {
    let normalize = |s: &str| s.replace('_', "").to_ascii_lowercase();
    let wanted = normalize(filter);
    let actual = normalize(kind.as_str());
    match wanted.as_str() {
        "endpoint" => matches!(kind, NodeKind::ApiEndpoint),
        "table" => matches!(kind, NodeKind::DatabaseTable),
        "test" => matches!(kind, NodeKind::TestCase | NodeKind::TestSuite),
        _ => wanted == actual,
    }
}

/// `GET .../symbols?q&kind&limit`: ranked exact > prefix > substring, at most 50.
pub async fn search(
    State(state): State<AppState>,
    claims: Authed,
    Path((repo, snap)): Path<(Uuid, Uuid)>,
    Query(query): Query<SymbolsQuery>,
) -> Result<Json<SymbolSearchResponse>, ApiError> {
    let scope = scope_of(&claims.0, repo)?;
    let limit = query.limit.unwrap_or(20);
    if limit == 0 || limit > MAX_SYMBOL_RESULTS {
        return Err(ApiError::BadRequest(format!(
            "limit must be 1..={MAX_SYMBOL_RESULTS}"
        )));
    }
    let needle = query
        .q
        .as_deref()
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty());
    let kind = query.kind.filter(|k| !k.trim().is_empty());
    let graph = state.graph(scope, SnapshotId::from_uuid(snap)).await?;
    let snapshot_id = snap.to_string();
    let limit = limit as usize;
    let response = state
        .blocking("symbols", move || {
            let mut candidates: Vec<(u8, String, NodeKey)> = Vec::new();
            let mut overflow = false;
            graph.for_each_node(&mut |node| {
                if candidates.len() >= MAX_CANDIDATES {
                    overflow = true;
                    return;
                }
                if let Some(filter) = &kind {
                    if !kind_matches(filter, node.kind) {
                        return;
                    }
                }
                let rank = match &needle {
                    None => 0,
                    Some(needle) => {
                        let name = node.name.to_lowercase();
                        let qualified = node.qualified_name.to_lowercase();
                        if name == *needle || qualified == *needle {
                            0
                        } else if name.starts_with(needle.as_str())
                            || qualified.starts_with(needle.as_str())
                        {
                            1
                        } else if name.contains(needle.as_str())
                            || qualified.contains(needle.as_str())
                        {
                            2
                        } else {
                            return;
                        }
                    }
                };
                candidates.push((rank, node.name.to_owned(), node.key));
            });
            candidates.sort_unstable();
            let truncated = overflow || candidates.len() > limit;
            candidates.truncate(limit);
            let degree = |key: NodeKey, dir: Direction| {
                u32::try_from(graph.degree(key, dir, &EdgeFilter::ALL)).unwrap_or(u32::MAX)
            };
            let items = candidates
                .into_iter()
                .filter_map(|(_, _, key)| {
                    let node = GraphQuery::node(&*graph, key)?;
                    Some(SymbolSearchItem {
                        key: key.to_string(),
                        name: node.name.to_owned(),
                        qualified_name: node.qualified_name.to_owned(),
                        kind: node.kind.as_str().to_owned(),
                        path: node.file.map(str::to_owned),
                        line: node.range.map(|r| r.start.line),
                        in_degree: degree(key, Direction::In),
                        out_degree: degree(key, Direction::Out),
                    })
                })
                .collect();
            Ok(SymbolSearchResponse {
                snapshot_id,
                items,
                truncated,
            })
        })
        .await?;
    Ok(Json(response))
}

/// `GET .../symbols/{key}`: the node, its edge counts per kind and a confidence histogram.
pub async fn detail(
    State(state): State<AppState>,
    claims: Authed,
    Path((repo, snap, raw_key)): Path<(Uuid, Uuid, String)>,
) -> Result<Json<SymbolDetail>, ApiError> {
    let scope = scope_of(&claims.0, repo)?;
    let key = parse_key(&raw_key);
    let graph = state.graph(scope, SnapshotId::from_uuid(snap)).await?;
    let snapshot_id = snap.to_string();
    let response = state
        .blocking("symbol", move || {
            let node = GraphQuery::node(&*graph, key).ok_or(ApiError::NotFound)?;
            let mut edges_in = BTreeMap::new();
            let mut edges_out = BTreeMap::new();
            let mut histogram = vec![0u32; 10];
            for (dir, counts) in [
                (Direction::Out, &mut edges_out),
                (Direction::In, &mut edges_in),
            ] {
                graph.for_each_edge(key, dir, &EdgeFilter::ALL, &mut |edge| {
                    bump(counts, edge.kind.as_str());
                    let bucket = usize::from(edge.confidence.as_permille() / 100).min(9);
                    if let Some(slot) = histogram.get_mut(bucket) {
                        *slot = slot.saturating_add(1);
                    }
                    ControlFlow::Continue(())
                });
            }
            // The node's own attribute strings live in the concrete graph's interner.
            let signature = node.attrs.signature.map(|id| graph.str(id).to_owned());
            let framework_facts = node
                .attrs
                .extra
                .as_ref()
                .map(|pairs| {
                    pairs
                        .iter()
                        .map(|(k, v)| FrameworkFact {
                            name: graph.str(*k).to_owned(),
                            value: graph.str(*v).to_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(SymbolDetail {
                snapshot_id,
                key: key.to_string(),
                id: node.id.to_owned(),
                name: node.name.to_owned(),
                qualified_name: node.qualified_name.to_owned(),
                kind: node.kind.as_str().to_owned(),
                path: node.file.map(str::to_owned),
                start_line: node.range.map(|r| r.start.line),
                end_line: node.range.map(|r| r.end.line),
                signature,
                visibility: format!("{:?}", node.attrs.visibility).to_ascii_lowercase(),
                framework_facts,
                edges_in,
                edges_out,
                confidence_histogram: histogram,
                truncated: false,
            })
        })
        .await?;
    Ok(Json(response))
}

/// `GET .../symbols/{key}/neighbors?dir&kinds&min_confidence`.
pub async fn neighbors(
    State(state): State<AppState>,
    claims: Authed,
    Path((repo, snap, raw_key)): Path<(Uuid, Uuid, String)>,
    Query(query): Query<NeighborsQuery>,
) -> Result<Json<NeighborsResponse>, ApiError> {
    let scope = scope_of(&claims.0, repo)?;
    let key = parse_key(&raw_key);
    let direction = match query.dir.as_deref() {
        None => Direction::Out,
        Some(raw) => Direction::from_str_exact(raw)
            .ok_or_else(|| ApiError::BadRequest("dir must be in, out or both".to_owned()))?,
    };
    let kinds: EdgeKindSet = match query.kinds.as_deref() {
        None => EdgeKindSet::ALL,
        Some(raw) => parse_kinds(raw.split(','))?,
    };
    let min_confidence = match query.min_confidence {
        None => Confidence::MIN,
        Some(value) if (0.0..=1.0).contains(&value) => Confidence::from_f32(value),
        Some(_) => {
            return Err(ApiError::BadRequest(
                "min_confidence must be between 0 and 1".to_owned(),
            ))
        }
    };
    let graph = state.graph(scope, SnapshotId::from_uuid(snap)).await?;
    let snapshot_id = snap.to_string();
    let response = state
        .blocking("neighbors", move || {
            let center = GraphQuery::node(&*graph, key).ok_or(ApiError::NotFound)?;
            let filter = EdgeFilter::new(kinds, min_confidence);
            let mut edges = Vec::new();
            let mut others: Vec<NodeKey> = Vec::new();
            let mut truncated = false;
            graph.for_each_edge(key, direction, &filter, &mut |edge| {
                if edges.len() >= MAX_NEIGHBOR_EDGES {
                    truncated = true;
                    return ControlFlow::Break(());
                }
                others.push(if edge.source == key {
                    edge.target
                } else {
                    edge.source
                });
                edges.push(edge_dto(&edge));
                ControlFlow::Continue(())
            });
            others.sort_unstable();
            others.dedup();
            let mut nodes = vec![node_dto(&center)];
            nodes.extend(
                others
                    .into_iter()
                    .filter(|other| *other != key)
                    .filter_map(|other| GraphQuery::node(&*graph, other))
                    .map(|node| node_dto(&node)),
            );
            Ok(NeighborsResponse {
                snapshot_id,
                key: key.to_string(),
                direction: direction.as_str().to_owned(),
                nodes,
                edges,
                truncated,
            })
        })
        .await?;
    Ok(Json(response))
}
