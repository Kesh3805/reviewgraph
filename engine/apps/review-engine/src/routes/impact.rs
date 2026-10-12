//! `GET /reviews/{run}/impact/{key}`: the impact of one changed symbol of a review run.
//!
//! The impact graph is computed by the pipeline and stored with the run; this route only
//! reshapes one seed's [`impact::SymbolImpact`] into the wire contract. The run must belong to
//! the repository in the token (the backend filters on organization and repository).

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::Json;
use impact::graph::{GraphSide, SeedSkip, TruncReason};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId};
use uuid::Uuid;

use super::{parse_key, Authed};
use crate::dto::{ImpactItemDto, ImpactResponse, ImpactStepDto, ImpactTruncationDto};
use crate::error::ApiError;
use crate::state::AppState;

fn side(side: GraphSide) -> &'static str {
    match side {
        GraphSide::Base => "base",
        GraphSide::Head => "head",
    }
}

fn skip(skipped: SeedSkip) -> &'static str {
    match skipped {
        SeedSkip::Cosmetic => "cosmetic",
        SeedSkip::Generated => "generated",
    }
}

fn reason(reason: TruncReason) -> &'static str {
    match reason {
        TruncReason::Limit => "limit",
        TruncReason::Depth => "depth",
        TruncReason::TotalCap => "total_cap",
        TruncReason::SeedMissing => "seed_missing",
    }
}

/// A readable name from a canonical node id: `ts:src/a#AuthService.authorize/method` gives
/// `AuthService.authorize`; ids without a `#` part are shown whole.
fn display_name(node_id: &str) -> String {
    match node_id.split_once('#') {
        Some((_, qualified)) => qualified
            .rsplit_once('/')
            .map_or(qualified, |(name, _)| name)
            .to_owned(),
        None => node_id.to_owned(),
    }
}

pub async fn impact(
    State(state): State<AppState>,
    claims: Authed,
    Path((run, raw_key)): Path<(Uuid, String)>,
) -> Result<Json<ImpactResponse>, ApiError> {
    let (Some(org), Some(repo)) = (claims.0.org, claims.0.repo) else {
        return Err(ApiError::Forbidden("missing_tenant_claims"));
    };
    let scope = RepoScope {
        organization_id: OrganizationId::from_uuid(org),
        repository_id: RepositoryId::from_uuid(repo),
    };
    let seed = parse_key(&raw_key);
    let graph = state
        .backend()
        .impact_graph(scope, run)
        .await?
        .ok_or(ApiError::NotFound)?;
    let symbol = graph.symbol(seed).ok_or(ApiError::NotFound)?;

    let mut relations: BTreeMap<String, Vec<ImpactItemDto>> = BTreeMap::new();
    for element in &symbol.elements {
        let relation = element.relation.as_str().to_owned();
        relations
            .entry(relation.clone())
            .or_default()
            .push(ImpactItemDto {
                key: element.node.to_string(),
                node_id: element.node_id.clone(),
                name: display_name(&element.node_id),
                kind: element.kind.as_str().to_owned(),
                relation,
                distance: element.distance,
                min_confidence: element.min_confidence.as_f32(),
                alt_paths: element.alt_paths,
                weak: element.weak,
                missing_on_head: element.missing_on_head,
                path: element
                    .path
                    .iter()
                    .map(|step| ImpactStepDto {
                        from: step.from.to_string(),
                        edge: step.edge.as_str().to_owned(),
                        to: step.to.to_string(),
                        confidence: step.confidence.as_f32(),
                        graph: side(step.graph).to_owned(),
                    })
                    .collect(),
            });
    }
    let truncation: Vec<ImpactTruncationDto> = symbol
        .truncation
        .iter()
        .map(|t| ImpactTruncationDto {
            relation: t.relation.map(|r| r.as_str().to_owned()),
            limit: t.limit,
            dropped: t.dropped,
            reason: reason(t.reason).to_owned(),
        })
        .collect();
    Ok(Json(ImpactResponse {
        review_run_id: run.to_string(),
        symbol_key: seed.to_string(),
        seed_id: symbol.seed_id.clone(),
        side: side(symbol.side).to_owned(),
        untested: symbol.untested,
        skipped: symbol.skipped.map(|s| skip(s).to_owned()),
        relations,
        truncated: !truncation.is_empty(),
        truncation,
    }))
}
