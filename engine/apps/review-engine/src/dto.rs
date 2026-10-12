//! Wire types of the internal API (`/internal/v1`).
//!
//! Responses derive `JsonSchema` so the contract can be exported for the TS client; node keys are
//! the 32-character lowercase hex form of `codegraph::NodeKey` and confidences are `0..=1`
//! floats. The API proxy (`apps/api/src/graph`) forwards these bodies unchanged, so the web
//! types in `apps/web/lib/api/pending.ts` mirror this file.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Budgets enforced by the engine, whatever the proxy already checked.
pub const MAX_SYMBOL_RESULTS: u32 = 50;
pub const MAX_SUBGRAPH_NODES: u32 = 500;
pub const MAX_SUBGRAPH_DEPTH: u8 = 3;
pub const MAX_SUBGRAPH_SEEDS: usize = 50;
pub const MAX_PATH_DEPTH: u8 = 6;
pub const MAX_PATHS: u32 = 10;
pub const DEFAULT_ALL_PATHS: u32 = 5;
pub const MAX_NEIGHBOR_EDGES: usize = 500;
pub const MAX_EXCERPT_LINES: u32 = 200;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GraphNodeDto {
    pub key: String,
    /// Canonical node id, e.g. `ts:src/a.ts#A.run/method`.
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    /// Repository-relative file, `None` for synthetic nodes.
    pub path: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GraphEdgeDto {
    pub source: String,
    pub target: String,
    pub kind: String,
    /// `0..=1`.
    pub confidence: f32,
    pub resolved_by: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolsQuery {
    pub q: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SymbolSearchItem {
    pub key: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub in_degree: u32,
    pub out_degree: u32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SymbolSearchResponse {
    pub snapshot_id: String,
    pub items: Vec<SymbolSearchItem>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FrameworkFact {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SymbolDetail {
    pub snapshot_id: String,
    pub key: String,
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub path: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub signature: Option<String>,
    pub visibility: String,
    pub framework_facts: Vec<FrameworkFact>,
    pub edges_in: BTreeMap<String, u32>,
    pub edges_out: BTreeMap<String, u32>,
    /// Ten buckets of incident edge confidence (`0.0-0.1` first).
    pub confidence_histogram: Vec<u32>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NeighborsQuery {
    /// `in`, `out` (default) or `both`.
    pub dir: Option<String>,
    /// Comma separated edge kinds, e.g. `CALLS,IMPORTS`.
    pub kinds: Option<String>,
    pub min_confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NeighborsResponse {
    pub snapshot_id: String,
    pub key: String,
    pub direction: String,
    pub nodes: Vec<GraphNodeDto>,
    pub edges: Vec<GraphEdgeDto>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubgraphRequest {
    pub seeds: Vec<String>,
    pub depth: Option<u8>,
    pub kinds: Option<Vec<String>>,
    pub max_nodes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SubgraphResponse {
    pub snapshot_id: String,
    pub nodes: Vec<GraphNodeDto>,
    pub edges: Vec<GraphEdgeDto>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathQuery {
    pub from: String,
    pub to: String,
    pub max_depth: Option<u8>,
    /// Comma separated edge kinds; every kind when absent.
    pub kinds: Option<String>,
    /// Return up to `max_paths` simple paths, shortest first, instead of the single shortest.
    pub all: Option<bool>,
    pub max_paths: Option<u32>,
}

/// An ordered chain: `nodes[0]` is the `from` node and `edges[i]` joins `nodes[i]` to
/// `nodes[i + 1]`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PathDto {
    pub nodes: Vec<GraphNodeDto>,
    pub edges: Vec<GraphEdgeDto>,
    /// The weakest edge of the path, `0..=1`.
    pub min_confidence: f32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PathResponse {
    pub snapshot_id: String,
    /// Shortest first. Empty when nothing is reachable within the budget.
    pub paths: Vec<PathDto>,
    /// The search budget ran out before it could say "no (more) paths".
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceQuery {
    pub path: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SourceResponse {
    pub snapshot_id: String,
    pub path: String,
    pub start: u32,
    pub end: u32,
    /// Redacted, at most 200 lines.
    pub text: String,
    pub redacted: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImpactStepDto {
    pub from: String,
    pub edge: String,
    pub to: String,
    pub confidence: f32,
    /// `base` or `head`.
    pub graph: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImpactItemDto {
    pub key: String,
    pub node_id: String,
    /// Derived from the canonical id.
    pub name: String,
    pub kind: String,
    /// The relation's snake_case name, e.g. `caller`, `endpoint`, `db_table`.
    pub relation: String,
    pub distance: u8,
    pub min_confidence: f32,
    pub alt_paths: u16,
    pub weak: bool,
    pub missing_on_head: bool,
    /// From the changed symbol to this element.
    pub path: Vec<ImpactStepDto>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImpactTruncationDto {
    pub relation: Option<String>,
    pub limit: u32,
    pub dropped: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImpactResponse {
    pub review_run_id: String,
    pub symbol_key: String,
    pub seed_id: String,
    pub side: String,
    pub untested: bool,
    pub skipped: Option<String>,
    /// Elements grouped by relation, in the impact graph's sort order.
    pub relations: BTreeMap<String, Vec<ImpactItemDto>>,
    pub truncation: Vec<ImpactTruncationDto>,
    pub truncated: bool,
}
