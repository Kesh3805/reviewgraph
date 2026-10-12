//! API profile and `endpoints_use_dto_validation` (PROF-005).
//!
//! Versioning is the majority route prefix (`/v1/...`, optionally below `/api`). DTO validation
//! is `global` when a validation edge is flagged `GLOBAL_SCOPE` (a global validation pipe),
//! `per_route` when handlers or their DTOs carry validation edges, otherwise `none`.

use std::collections::BTreeMap;

use codegraph::{EdgeFlags, EdgeKind, EdgeKindSet, GraphQueryExt, NodeKey, NodeKind};

use super::{collect, out_targets};
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
use crate::model::ApiProfile;

pub const ID: &str = "endpoints_use_dto_validation";

#[derive(Debug, Default, Clone, Copy)]
pub struct EndpointsUseDtoValidation;

/// `v1` for `/v1/users` and `/api/v1/users`.
fn version_prefix(path: &str) -> Option<String> {
    let mut segments = path.trim_start_matches('/').split('/');
    let mut first = segments.next()?;
    if first == "api" {
        first = segments.next()?;
    }
    let digits = first.strip_prefix('v')?;
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())).then(|| first.to_owned())
}

/// The path of an endpoint whose qualified name is `GET /v1/users`.
fn route_path(qualified_name: &str) -> &str {
    qualified_name
        .split_once(' ')
        .map_or(qualified_name, |(_, path)| path)
}

/// Whether a validation edge applies repository-wide.
fn has_global_validation(g: &dyn GraphView) -> bool {
    let q = g.graph();
    let validates = EdgeKindSet::of(EdgeKind::Validates);
    collect(q, |_| true).iter().any(|n| {
        q.out_edges(n.key, validates)
            .iter()
            .any(|e| e.flags.contains(EdgeFlags::GLOBAL_SCOPE))
    })
}

fn touches_validation(g: &dyn GraphView, key: NodeKey) -> bool {
    let q = g.graph();
    let validates = EdgeKindSet::of(EdgeKind::Validates);
    !q.out_edges(key, validates).is_empty() || !q.in_edges(key, validates).is_empty()
}

/// Versioning and DTO validation of the HTTP surface.
pub fn infer_api(g: &dyn GraphView) -> ApiProfile {
    let q = g.graph();
    let endpoints = collect(q, |n| n.kind == NodeKind::ApiEndpoint);
    if endpoints.is_empty() {
        return ApiProfile::default();
    }
    let mut prefixes: BTreeMap<Option<String>, usize> = BTreeMap::new();
    for e in &endpoints {
        *prefixes
            .entry(version_prefix(route_path(&e.qualified_name)))
            .or_insert(0) += 1;
    }
    let total = endpoints.len();
    let versioned: usize = prefixes
        .iter()
        .filter(|(k, _)| k.is_some())
        .map(|(_, n)| *n)
        .sum();
    let (versioning, share) = if versioned * 2 >= total {
        let best = prefixes
            .iter()
            .filter(|(k, _)| k.is_some())
            .map(|(_, n)| *n)
            .max()
            .unwrap_or(0);
        ("path", best)
    } else {
        ("none", total - versioned)
    };

    let dto_validation = if has_global_validation(g) {
        "global"
    } else if endpoints.iter().any(|e| {
        out_targets(q, e.key, EdgeKindSet::of(EdgeKind::HandledBy))
            .into_iter()
            .any(|h| touches_validation(g, h))
    }) {
        "per_route"
    } else {
        "none"
    };
    ApiProfile {
        style: Some("http".to_owned()),
        versioning: Some(versioning.to_owned()),
        versioning_consistency: share as f32 / total as f32,
        dto_validation: Some(dto_validation.to_owned()),
        response_envelope: None,
    }
}

impl ConventionMiner for EndpointsUseDtoValidation {
    fn id(&self) -> &'static str {
        ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "Endpoints that accept a typed body validate it (a global pipe or per-route validation)."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        let q = g.graph();
        let global = has_global_validation(g);
        let mut samples: Vec<Sample> = collect(q, |n| n.kind == NodeKind::ApiEndpoint)
            .into_iter()
            .map(|e| {
                let path = out_targets(q, e.key, EdgeKindSet::of(EdgeKind::HandledBy))
                    .into_iter()
                    .filter_map(|h| q.node(h).and_then(|n| n.file.map(str::to_owned)))
                    .next()
                    .unwrap_or_default();
                let meta = format!("{}|{}", if global { "global" } else { "" }, e.name);
                Sample::new(e.id, path).with_meta(meta)
            })
            .collect();
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        samples
    }

    fn conforms(&self, s: &Sample, g: &dyn GraphView) -> Conformance {
        let q = g.graph();
        let Some(endpoint) = q.node_by_id(&s.id) else {
            return Conformance::NotApplicable;
        };
        let (global, name) = s.meta.split_once('|').unwrap_or(("", &s.meta));
        let handlers = out_targets(q, endpoint.key, EdgeKindSet::of(EdgeKind::HandledBy));
        let accepted: Vec<NodeKey> = handlers
            .iter()
            .flat_map(|h| out_targets(q, *h, EdgeKindSet::of(EdgeKind::AcceptsType)))
            .collect();
        if accepted.is_empty() {
            return Conformance::NotApplicable;
        }
        let validated = !global.is_empty()
            || handlers
                .iter()
                .chain(accepted.iter())
                .any(|k| touches_validation(g, *k));
        if validated {
            Conformance::Conforms
        } else {
            Conformance::Violates {
                evidence: format!("endpoint {name} accepts a DTO that is never validated"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgraph::{partial, OwnedView, TestGraph};

    fn endpoints(t: &mut TestGraph, routes: &[(&str, &str)]) {
        for (i, (method, path)) in routes.iter().enumerate() {
            let ep = format!("http:{method}:{path}");
            let handler = format!("m:{i}");
            t.node(
                NodeKind::ApiEndpoint,
                &ep,
                &format!("{method} {path}"),
                None,
            )
            .node(
                NodeKind::Method,
                &handler,
                "h",
                Some("src/a/a.controller.ts"),
            )
            .edge(EdgeKind::HandledBy, &ep, &handler);
        }
    }

    #[test]
    fn api_versioning_majority() {
        let mut t = TestGraph::new();
        endpoints(
            &mut t,
            &[
                ("GET", "/v1/a"),
                ("POST", "/v1/a"),
                ("GET", "/v1/b"),
                ("GET", "/health"),
            ],
        );
        let p = infer_api(&OwnedView::new(t.build()));
        assert_eq!(p.versioning.as_deref(), Some("path"));
        assert!((p.versioning_consistency - 0.75).abs() < 1e-6);
        assert_eq!(p.dto_validation.as_deref(), Some("none"));
    }

    #[test]
    fn dto_validation_global_pipe() {
        let mut t = TestGraph::new();
        endpoints(&mut t, &[("POST", "/v1/a")]);
        t.node(
            NodeKind::Class,
            "c:pipe",
            "ValidationPipe",
            Some("src/main.ts"),
        )
        .node(NodeKind::Class, "c:dto", "CreateA", Some("src/a/a.dto.ts"))
        .edge_flagged(
            EdgeKind::Validates,
            "c:pipe",
            "c:dto",
            EdgeFlags::GLOBAL_SCOPE,
        )
        .edge(EdgeKind::AcceptsType, "m:0", "c:dto");
        let v = OwnedView::new(t.build());
        assert_eq!(infer_api(&v).dto_validation.as_deref(), Some("global"));
        let samples = EndpointsUseDtoValidation.population(&v, &partial());
        assert_eq!(
            EndpointsUseDtoValidation.conforms(&samples[0], &v),
            Conformance::Conforms
        );
    }

    #[test]
    fn unvalidated_dto_violates_and_untyped_is_not_applicable() {
        let mut t = TestGraph::new();
        endpoints(&mut t, &[("POST", "/v1/a"), ("GET", "/v1/b")]);
        t.node(NodeKind::Class, "c:dto", "CreateA", Some("src/a/a.dto.ts"))
            .edge(EdgeKind::AcceptsType, "m:0", "c:dto");
        let v = OwnedView::new(t.build());
        let samples = EndpointsUseDtoValidation.population(&v, &partial());
        let verdict = |id: &str| {
            let s = samples.iter().find(|s| s.id == id).unwrap();
            EndpointsUseDtoValidation.conforms(s, &v)
        };
        assert!(matches!(
            verdict("http:POST:/v1/a"),
            Conformance::Violates { .. }
        ));
        assert_eq!(verdict("http:GET:/v1/b"), Conformance::NotApplicable);
    }
}
