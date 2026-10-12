//! API profile and `endpoints_use_dto_validation` (PROF-005).
//!
//! Versioning is the majority route prefix (`/v\d+/`), recorded with its consistency. DTO
//! validation is `global` when a global validation pipe is registered, `per_route` when handlers
//! validate their input, and `none` otherwise.

use analysis_ir::facts::FactKind;
use codegraph::{Direction, EdgeKind, NodeKind};

use super::{edges, handler_of, node_sample, nodes_of_kind};
use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::facts::detail_str;
use crate::infer::architecture::round3;
use crate::infer::roles::Role;
use crate::model::{ApiVersioning, DtoValidation, RepositoryProfile};

pub const ID: &str = "endpoints_use_dto_validation";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default)]
pub struct ApiMiner;

/// The route path of an endpoint id `http:GET /v1/users`.
pub fn route_path(endpoint_id: &str) -> Option<&str> {
    endpoint_id
        .strip_prefix("http:")
        .and_then(|rest| rest.split_once(' '))
        .map(|(_, path)| path)
}

/// Whether a route is path-versioned (`/v1/...`, `/api/v2/...`).
pub fn is_path_versioned(path: &str) -> bool {
    path.split('/').any(|segment| {
        let mut chars = segment.chars();
        matches!(chars.next(), Some('v' | 'V'))
            && segment.len() > 1
            && chars.all(|c| c.is_ascii_digit())
    })
}

/// A global validation pipe is registered anywhere.
pub fn has_global_validation(cx: &MiningContext<'_>) -> bool {
    cx.inputs.facts.iter().any(|(_, facts)| {
        facts.iter().any(|f| {
            f.kind == FactKind::Call
                && detail_str(f, "callee").is_some_and(|c| c.ends_with("useGlobalPipes"))
        })
    })
}

fn handler_validates(cx: &MiningContext<'_>, endpoint: codegraph::NodeKey) -> bool {
    let graph = cx.inputs.graph;
    let Some(handler) = handler_of(graph, endpoint) else {
        return false;
    };
    if !edges(graph, handler.key, Direction::Out, &[EdgeKind::Validates]).is_empty() {
        return true;
    }
    edges(graph, handler.key, Direction::Out, &[EdgeKind::AcceptsType])
        .iter()
        .filter_map(|e| graph.node(e.target))
        .any(|t| cx.roles.node_role(&t) == Some(Role::Dto))
}

impl ConventionMiner for ApiMiner {
    fn id(&self) -> &str {
        ID
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        "API endpoints validate their input through DTOs".to_owned()
    }

    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample> {
        let graph = cx.inputs.graph;
        nodes_of_kind(graph, &[NodeKind::ApiEndpoint])
            .into_iter()
            .filter_map(|endpoint| {
                let handler = handler_of(graph, endpoint.key)?;
                let mut sample = node_sample(&handler, endpoint.id)?;
                sample.key = Some(endpoint.key);
                Some(sample)
            })
            .collect()
    }

    fn conforms(&self, sample: &Sample, cx: &MiningContext<'_>) -> Conformance {
        let Some(endpoint) = sample.key else {
            return Conformance::NotApplicable;
        };
        if has_global_validation(cx) || handler_validates(cx, endpoint) {
            Conformance::Conforms
        } else {
            Conformance::Violates {
                evidence: format!("endpoint `{}` does not validate its input", sample.detail),
            }
        }
    }

    fn contribute(&self, cx: &MiningContext<'_>, profile: &mut RepositoryProfile) {
        let graph = cx.inputs.graph;
        let endpoints = nodes_of_kind(graph, &[NodeKind::ApiEndpoint]);
        let total = endpoints.len();
        let versioned = endpoints
            .iter()
            .filter(|e| route_path(e.id).is_some_and(is_path_versioned))
            .count();
        let share = if total == 0 {
            0.0
        } else {
            versioned as f32 / total as f32
        };
        profile.api.endpoints = u32::try_from(total).unwrap_or(u32::MAX);
        if total > 0 && share >= 0.5 {
            profile.api.versioning = ApiVersioning::Path;
            profile.api.versioning_consistency = round3(share);
        } else {
            profile.api.versioning = ApiVersioning::None;
            profile.api.versioning_consistency = if total == 0 { 0.0 } else { round3(1.0 - share) };
        }
        profile.api.dto_validation = if has_global_validation(cx) {
            DtoValidation::Global
        } else if endpoints.iter().any(|e| handler_validates(cx, e.key)) {
            DtoValidation::PerRoute
        } else {
            DtoValidation::None
        };
    }
}
