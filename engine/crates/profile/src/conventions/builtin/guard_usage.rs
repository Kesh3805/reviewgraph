//! `endpoints_are_guarded` (PROF-004).
//!
//! The population is endpoints. An endpoint conforms when a guard authorizes it: per method, per
//! class, or globally (a global guard is an `AUTHORIZES` edge flagged `GLOBAL_SCOPE`). The miner
//! also records the authorization symbols: what guarded handlers call first.

use std::collections::BTreeMap;

use analysis_ir::facts::FactKind;
use codegraph::{EdgeKind, EdgeKindSet, NodeKind};

use super::{collect, in_sources, out_targets};
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};

pub const ID: &str = "endpoints_are_guarded";

/// An authorization symbol must lead at least this many guarded handlers to be reported.
const MIN_AUTH_SYMBOL_USES: usize = 2;
/// At most this many authorization symbols are reported.
const MAX_AUTH_SYMBOLS: usize = 5;

#[derive(Debug, Default, Clone, Copy)]
pub struct GuardUsage;

impl ConventionMiner for GuardUsage {
    fn id(&self) -> &'static str {
        ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "Every endpoint is protected by a guard (per route, per controller or global)."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        let q = g.graph();
        let mut samples: Vec<Sample> = collect(q, |n| n.kind == NodeKind::ApiEndpoint)
            .into_iter()
            .map(|e| {
                let handler_file = out_targets(q, e.key, EdgeKindSet::of(EdgeKind::HandledBy))
                    .into_iter()
                    .filter_map(|h| q.node(h).and_then(|n| n.file.map(str::to_owned)))
                    .next()
                    .unwrap_or_default();
                Sample::new(e.id, handler_file).with_meta(e.name)
            })
            .collect();
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        samples
    }

    fn conforms(&self, s: &Sample, g: &dyn GraphView) -> Conformance {
        let q = g.graph();
        let Some(node) = q.node_by_id(&s.id) else {
            return Conformance::NotApplicable;
        };
        if in_sources(q, node.key, EdgeKindSet::of(EdgeKind::Authorizes)).is_empty() {
            Conformance::Violates {
                evidence: format!("endpoint {} has no guard", s.meta),
            }
        } else {
            Conformance::Conforms
        }
    }
}

/// The callee of the first `call` fact of `node_id`, from the key `call:<callee>/<arity>`.
fn first_callee(g: &dyn GraphView, node_id: &str) -> Option<String> {
    let fact = g.facts(node_id).iter().find(|f| f.kind == FactKind::Call)?;
    let rest = fact.key.strip_prefix("call:")?;
    let callee = rest.rsplit_once('/').map_or(rest, |(callee, _)| callee);
    (!callee.is_empty()).then(|| callee.to_owned())
}

/// The symbols most frequently called first in guarded handlers (consumed by REV-S-003): most
/// frequent first, ties by name.
pub fn authorization_symbols(g: &dyn GraphView) -> Vec<String> {
    let q = g.graph();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for endpoint in collect(q, |n| n.kind == NodeKind::ApiEndpoint) {
        if in_sources(q, endpoint.key, EdgeKindSet::of(EdgeKind::Authorizes)).is_empty() {
            continue;
        }
        for handler in out_targets(q, endpoint.key, EdgeKindSet::of(EdgeKind::HandledBy)) {
            let Some(handler) = q.node(handler) else {
                continue;
            };
            if let Some(callee) = first_callee(g, handler.id) {
                *counts.entry(callee).or_insert(0) += 1;
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, n)| *n >= MIN_AUTH_SYMBOL_USES)
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(MAX_AUTH_SYMBOLS)
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgraph::{fact, partial, OwnedView, TestGraph};
    use codegraph::EdgeFlags;

    fn view() -> OwnedView {
        let mut t = TestGraph::new();
        t.node(
            NodeKind::Middleware,
            "g:auth",
            "AuthGuard",
            Some("src/auth/auth.guard.ts"),
        )
        .node(
            NodeKind::Middleware,
            "g:global",
            "GlobalGuard",
            Some("src/app.guard.ts"),
        );
        for (i, name) in ["h1", "h2", "h3", "h4"].iter().enumerate() {
            let ep = format!("http:GET:/{name}");
            let handler = format!("m:{name}");
            t.node(NodeKind::ApiEndpoint, &ep, &format!("GET /{name}"), None)
                .node(
                    NodeKind::Method,
                    &handler,
                    name,
                    Some("src/x/x.controller.ts"),
                )
                .edge(EdgeKind::HandledBy, &ep, &handler);
            match i {
                0 | 1 => {
                    t.edge(EdgeKind::Authorizes, "g:auth", &ep);
                }
                2 => {
                    t.edge_flagged(
                        EdgeKind::Authorizes,
                        "g:global",
                        &ep,
                        EdgeFlags::GLOBAL_SCOPE,
                    );
                }
                _ => {}
            }
        }
        let call = || vec![fact(FactKind::Call, "call:this.permissions.check/1", &[])];
        OwnedView::new(t.build())
            .with_facts("m:h1", call())
            .with_facts("m:h2", call())
            .with_facts(
                "m:h3",
                vec![fact(FactKind::Call, "call:this.other.go/0", &[])],
            )
    }

    #[test]
    fn guard_global_counts_as_conforming() {
        let v = view();
        let samples = GuardUsage.population(&v, &partial());
        assert_eq!(samples.len(), 4);
        let verdicts: Vec<(String, Conformance)> = samples
            .iter()
            .map(|s| (s.id.clone(), GuardUsage.conforms(s, &v)))
            .collect();
        let conforming = verdicts
            .iter()
            .filter(|(_, c)| *c == Conformance::Conforms)
            .count();
        assert_eq!(conforming, 3, "per-route and global guards both conform");
        assert!(verdicts.iter().any(|(id, c)| id == "http:GET:/h4"
            && matches!(c, Conformance::Violates { evidence } if evidence.contains("GET /h4"))));
    }

    #[test]
    fn authorization_symbols_extracted() {
        let v = view();
        assert_eq!(
            authorization_symbols(&v),
            vec!["this.permissions.check".to_owned()]
        );
    }
}
