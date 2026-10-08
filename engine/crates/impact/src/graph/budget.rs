//! Impact budgets (IMP-001 defaults).
//!
//! Every expansion takes an explicit budget and reports when it stopped short; nothing in the
//! impact graph is unbounded (master plan principle 4).

use codegraph::Confidence;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Caps for one impact build. Element caps are per seed unless named `_pr`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactBudget {
    /// Reverse `CALLS` depth. Depth 3 additionally requires half the PR budget to remain
    /// after every seed's depth-2 pass (IMP-002).
    pub max_caller_depth: u8,
    pub max_callers: u32,
    pub max_callees: u32,
    /// Removed callees, implementations, interfaces, overrides, sub/supertypes and related types.
    pub max_type_relations: u32,
    pub max_endpoints: u32,
    pub max_endpoint_depth: u8,
    /// Nodes the endpoint search may visit per seed (visited nodes are not elements).
    pub max_endpoint_visits: u32,
    pub max_tests: u32,
    /// Resource elements at distance 1 (tables, queues, config, env vars, external APIs).
    pub max_resources: u32,
    /// "Other side" resource elements per resource (co-writers, other readers, consumers).
    pub max_resource_other_side: u32,
    pub max_total_elements_per_symbol: u32,
    pub max_total_elements_pr: u32,
    /// Paths weaker than this are listed as `weak` and never expanded further.
    pub min_confidence: Confidence,
}

impl Default for ImpactBudget {
    fn default() -> Self {
        Self {
            max_caller_depth: 2,
            max_callers: 50,
            max_callees: 30,
            max_type_relations: 30,
            max_endpoints: 10,
            max_endpoint_depth: 6,
            max_endpoint_visits: 2_000,
            max_tests: 20,
            max_resources: 30,
            max_resource_other_side: 5,
            max_total_elements_per_symbol: 200,
            max_total_elements_pr: 5_000,
            min_confidence: Confidence::from_f32(0.5),
        }
    }
}
