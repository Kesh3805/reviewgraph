//! `endpoints_are_guarded` (PROF-004).
//!
//! The population is every API endpoint with a handler. An endpoint conforms when a guard
//! `AUTHORIZES` it (method, class or global scope) or its handler or controller carries a guard
//! decorator fact. The miner also records `authorization_symbols`: the symbols most frequently
//! called first in guarded handlers (consumed by REV-S-003).

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::facts::FactKind;
use codegraph::{Direction, EdgeFlags, EdgeKind, GraphQuery, NodeKey, NodeKind};

use super::{edges, handler_of, node_sample, nodes_of_kind};
use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::model::RepositoryProfile;

pub const ID: &str = "endpoints_are_guarded";
pub const VERSION: u32 = 1;
/// Fewest guarded handlers that must call a symbol first for it to count as authorization.
const MIN_AUTH_CALLS: u32 = 2;
const MAX_AUTH_SYMBOLS: usize = 10;

#[derive(Debug, Clone, Copy, Default)]
pub struct GuardUsageMiner;

/// Whether an endpoint is guarded.
pub fn is_guarded(cx: &MiningContext<'_>, endpoint: NodeKey) -> bool {
    let graph = cx.inputs.graph;
    if !edges(graph, endpoint, Direction::In, &[EdgeKind::Authorizes]).is_empty() {
        return true;
    }
    let Some(handler) = handler_of(graph, endpoint) else {
        return false;
    };
    let facts = cx.inputs.facts;
    if facts.facts_of_kind(handler.key, FactKind::GuardDecorator).next().is_some() {
        return true;
    }
    handler
        .attrs
        .parent
        .is_some_and(|class| facts.facts_of_kind(class, FactKind::GuardDecorator).next().is_some())
}

/// The first `CALLS` target of a handler, by call-site line.
fn first_call(graph: &dyn GraphQuery, handler: NodeKey) -> Option<String> {
    edges(graph, handler, Direction::Out, &[EdgeKind::Calls])
        .into_iter()
        .min_by_key(|e| e.location.map_or(u32::MAX, |(_, line, _)| line))
        .and_then(|e| graph.node(e.target))
        .map(|n| n.qualified_name.to_owned())
}

impl ConventionMiner for GuardUsageMiner {
    fn id(&self) -> &str {
        ID
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        "Every API endpoint is protected by a guard".to_owned()
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
        if is_guarded(cx, endpoint) {
            Conformance::Conforms
        } else {
            Conformance::Violates {
                evidence: format!("endpoint `{}` has no guard", sample.detail),
            }
        }
    }

    fn contribute(&self, cx: &MiningContext<'_>, profile: &mut RepositoryProfile) {
        let graph = cx.inputs.graph;
        let mut guarded = 0u32;
        let mut unguarded = 0u32;
        let mut global = BTreeSet::new();
        let mut first_calls: BTreeMap<String, u32> = BTreeMap::new();
        for endpoint in nodes_of_kind(graph, &[NodeKind::ApiEndpoint]) {
            for edge in edges(graph, endpoint.key, Direction::In, &[EdgeKind::Authorizes]) {
                if edge.flags.contains(EdgeFlags::GLOBAL_SCOPE) {
                    if let Some(guard) = graph.node(edge.source) {
                        global.insert(guard.qualified_name.to_owned());
                    }
                }
            }
            if is_guarded(cx, endpoint.key) {
                guarded += 1;
                if let Some(symbol) =
                    handler_of(graph, endpoint.key).and_then(|h| first_call(graph, h.key))
                {
                    *first_calls.entry(symbol).or_default() += 1;
                }
            } else {
                unguarded += 1;
            }
        }
        let mut ranked: Vec<(String, u32)> = first_calls
            .into_iter()
            .filter(|(_, n)| *n >= MIN_AUTH_CALLS)
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        profile.security.guarded_endpoints = guarded;
        profile.security.unguarded_endpoints = unguarded;
        profile.security.global_guards = global.into_iter().collect();
        profile.security.authorization_symbols = ranked
            .into_iter()
            .take(MAX_AUTH_SYMBOLS)
            .map(|(s, _)| s)
            .collect();
    }
}
