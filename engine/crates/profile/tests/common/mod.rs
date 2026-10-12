//! Shared test helpers.

#![allow(dead_code)]

pub mod graphs;
#[cfg(feature = "integration")]
pub mod pg;

use chrono::{TimeZone, Utc};
use codegraph::Graph;
use profile::config::ReviewConfigV1;
use profile::conventions::miner::Classified;
use profile::conventions::{mine, ConventionMiner, MiningContext};
use profile::facts::FactIndex;
use profile::infer::architecture::infer_architecture;
use profile::infer::roles::RoleIndex;
use profile::infer::ProfileInputs;
use profile::model::Convention;
use review_core::version::ProfileVersion;

pub fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap()
}

/// Runs one miner over a graph with the default config.
pub fn mine_one(graph: &Graph, facts: &FactIndex, miner: &dyn ConventionMiner) -> Option<Convention> {
    mine_with(graph, facts, &ReviewConfigV1::default(), miner)
}

pub fn mine_with(
    graph: &Graph,
    facts: &FactIndex,
    config: &ReviewConfigV1,
    miner: &dyn ConventionMiner,
) -> Option<Convention> {
    with_context(graph, facts, config, |cx| mine(miner, cx, ProfileVersion(1), now()))
}

pub fn classify_one(graph: &Graph, facts: &FactIndex, miner: &dyn ConventionMiner) -> Classified {
    with_context(graph, facts, &ReviewConfigV1::default(), |cx| {
        profile::conventions::miner::classify(miner, cx)
    })
}

/// Builds a mining context (roles, modules) and hands it to `f`.
pub fn with_context<T>(
    graph: &Graph,
    facts: &FactIndex,
    config: &ReviewConfigV1,
    f: impl FnOnce(&MiningContext<'_>) -> T,
) -> T {
    let inputs = ProfileInputs::graph_only(graph, facts, config);
    let roles = RoleIndex::build(graph, &[]);
    let architecture = infer_architecture(&inputs, &roles);
    let modules = architecture.module_index();
    let cx = MiningContext::new(inputs, &roles, &modules);
    f(&cx)
}
