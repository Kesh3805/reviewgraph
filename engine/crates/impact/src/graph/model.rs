//! The impact graph model (IMP-001).
//!
//! An [`ImpactGraph`] is the derived neighbourhood of a pull request's changed symbols: for every
//! seed, the elements it reaches, *why* each one is included ([`Relation`]), how far away it is
//! and how trustworthy the path is (target-architecture §3.7). Reviewers, context selection and
//! verification all read the same value, so ordering and tie-breaking are part of the contract:
//! two builds over the same inputs serialize byte-identically.

use std::collections::BTreeMap;

use codegraph::{Confidence, EdgeKind, NodeKey, NodeKind};
use review_core::ids::SymbolKey;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::budget::ImpactBudget;

/// Bumped when the expansion algorithms change in a way that changes results. Part of
/// [`ImpactGraph::input_hash`].
pub const IMPACT_VERSION: u16 = 1;

/// Bumped when a field of the serialized [`ImpactGraph`] is removed or renamed.
pub const IMPACT_SCHEMA_VERSION: u16 = 1;

/// Why an element belongs to a seed's impact. The declaration order is the sort order of
/// [`SymbolImpact::elements`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Calls the seed (reverse `CALLS`, possibly through interface dispatch).
    Caller,
    /// Called by the seed on the head graph.
    Callee,
    /// Called by the seed on the base graph, and no longer called on head.
    RemovedCallee,
    /// Implements the same contract as the seed, or implements the seed (an interface member).
    Implementation,
    /// The interface member the seed implements.
    Interface,
    /// The supertype member the seed overrides.
    Override,
    /// A subtype member overriding the seed.
    OverriddenBy,
    /// A type extending or implementing the seed type.
    Subtype,
    /// A type the seed type extends or implements.
    Supertype,
    /// A symbol using, accepting or returning the seed type.
    RelatedType,
    /// A public entrypoint (HTTP route, queue consumer, CLI command, cron job, worker).
    Endpoint,
    /// A test exercising (or mocking) the seed.
    Test,
    /// A configuration node the seed reads or writes.
    Config,
    /// An environment variable the seed reads, or another reader of it.
    EnvVar,
    /// A table the seed reads or writes, or another writer/reader of it.
    DbTable,
    /// An ORM entity the seed's container class injects.
    DbEntity,
    /// A queue the seed produces to.
    QueueProducer,
    /// A consumer of a queue the seed produces to, or a queue the seed consumes.
    QueueConsumer,
    /// An external API or dependency the seed depends on.
    ExternalApi,
    /// The class or module containing the seed.
    Container,
}

impl Relation {
    /// Every relation, in sort order.
    pub const ALL: [Relation; 20] = [
        Self::Caller,
        Self::Callee,
        Self::RemovedCallee,
        Self::Implementation,
        Self::Interface,
        Self::Override,
        Self::OverriddenBy,
        Self::Subtype,
        Self::Supertype,
        Self::RelatedType,
        Self::Endpoint,
        Self::Test,
        Self::Config,
        Self::EnvVar,
        Self::DbTable,
        Self::DbEntity,
        Self::QueueProducer,
        Self::QueueConsumer,
        Self::ExternalApi,
        Self::Container,
    ];

    /// The snake_case wire name, also used as the `relation` metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Caller => "caller",
            Self::Callee => "callee",
            Self::RemovedCallee => "removed_callee",
            Self::Implementation => "implementation",
            Self::Interface => "interface",
            Self::Override => "override",
            Self::OverriddenBy => "overridden_by",
            Self::Subtype => "subtype",
            Self::Supertype => "supertype",
            Self::RelatedType => "related_type",
            Self::Endpoint => "endpoint",
            Self::Test => "test",
            Self::Config => "config",
            Self::EnvVar => "env_var",
            Self::DbTable => "db_table",
            Self::DbEntity => "db_entity",
            Self::QueueProducer => "queue_producer",
            Self::QueueConsumer => "queue_consumer",
            Self::ExternalApi => "external_api",
            Self::Container => "container",
        }
    }

    /// The plural, human-readable label used in the truncation summary line.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Caller => "callers",
            Self::Callee => "callees",
            Self::RemovedCallee => "removed callees",
            Self::Implementation => "implementations",
            Self::Interface => "interfaces",
            Self::Override => "overrides",
            Self::OverriddenBy => "overriders",
            Self::Subtype => "subtypes",
            Self::Supertype => "supertypes",
            Self::RelatedType => "related types",
            Self::Endpoint => "endpoints",
            Self::Test => "tests",
            Self::Config => "config",
            Self::EnvVar => "env vars",
            Self::DbTable => "tables",
            Self::DbEntity => "entities",
            Self::QueueProducer => "queue producers",
            Self::QueueConsumer => "queue consumers",
            Self::ExternalApi => "external APIs",
            Self::Container => "containers",
        }
    }
}

/// Which graph a path step was read from. Head is the default; removed callees and the
/// impact of removed symbols are read from base.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GraphSide {
    Base,
    #[default]
    Head,
}

/// One edge of an element's path.
///
/// `from`/`to` are the edge as stored (`from` is the edge source), so a verifier can re-check
/// the step with a single `for_each_edge` lookup; the path as a whole is ordered from the seed
/// to the element. A synthesized interface-dispatch hop is recorded as an `IMPLEMENTS` step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathStep {
    pub from: NodeKey,
    pub edge: EdgeKind,
    pub to: NodeKey,
    /// The confidence of this edge (CG-003 table), in permille.
    pub confidence: Confidence,
    pub graph: GraphSide,
}

/// What kind of entrypoint an [`Relation::Endpoint`] element is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Http,
    Queue,
    Cli,
    Cron,
    Worker,
}

/// Attributes of an endpoint element. Guards are recorded, never evaluated (VER-007 does that).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EndpointAttrs {
    pub entry_kind: EntryKind,
    /// HTTP method, for `http` entrypoints.
    pub method: Option<String>,
    /// Route path as declared (`/users/:id`), or the queue name for `queue` entrypoints.
    pub path: Option<String>,
    /// Names of the guards with an `AUTHORIZES` edge onto the endpoint, sorted.
    pub guards: Vec<String>,
}

/// The individual signals behind a test mapping, each in `[0, 1]` (IMP-005).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestSignals {
    pub invocation: f32,
    pub tests_edge: f32,
    pub import: f32,
    pub naming: f32,
    pub path: f32,
    pub mock: f32,
}

/// Why a test element is attached to a seed (IMP-005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestMapping {
    /// `1 − Π(1 − signal)`, rounded to four decimals.
    pub score: f32,
    pub signals: TestSignals,
    /// The test depends on the seed's class through a mock and does not exercise it.
    pub mocked: bool,
}

/// How a resource element relates to the seed (IMP-006).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ResourceRole {
    Reads,
    Writes,
    Produces,
    Consumes,
    Publishes,
    Subscribes,
    DependsOn,
    /// An entity injected into the seed's container class.
    Injected,
    /// Another writer of a table the seed writes.
    CoWriter,
    /// Another reader of a table or environment variable the seed touches.
    CoReader,
    /// A consumer of a queue the seed produces to.
    Consumer,
}

/// Attributes of a resource element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceAttrs {
    pub role: ResourceRole,
    /// The resource edge exists on base only: the change removed it.
    pub removed: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One element of a seed's impact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactElement {
    pub node: NodeKey,
    pub node_id: String,
    pub kind: NodeKind,
    pub relation: Relation,
    /// Hops from the seed. Interface-dispatch hops do not count.
    pub distance: u8,
    /// The best path from the seed to `node` (see [`super::path::compare_trails`]).
    pub path: Vec<PathStep>,
    /// The weakest confidence along `path`.
    pub min_confidence: Confidence,
    /// How many other paths reached the same element under the same relation.
    pub alt_paths: u16,
    /// `min_confidence` is below the budget's `min_confidence`: listed, never expanded further.
    pub weak: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<EndpointAttrs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestMapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceAttrs>,
    /// For callers: resource node ids the caller itself reads or writes (tables, queues), so a
    /// caller's table shows up on the caller rather than as a seed relation (IMP-006).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touches: Vec<String>,
    /// For callers of a removed seed (computed on base): the caller no longer exists on head.
    #[serde(default, skip_serializing_if = "is_false")]
    pub missing_on_head: bool,
}

/// Why a seed has no expansion at all.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SeedSkip {
    /// Formatting or comment-only change (CHG-001 `cosmetic`).
    Cosmetic,
    /// The symbol lives in a generated or vendored file.
    Generated,
}

/// Why a relation (or a whole seed) stopped short.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TruncReason {
    /// The relation's own cap was reached.
    Limit,
    /// The depth (or visit) budget ended the search while work remained.
    Depth,
    /// The seed's or the pull request's total element cap was reached.
    TotalCap,
    /// The seed is not a node of the graph it should be resolved on (degraded parse).
    SeedMissing,
}

/// A budget stop, always reported (master plan principle 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Truncation {
    /// `None` for a seed-level stop such as [`TruncReason::SeedMissing`].
    pub relation: Option<Relation>,
    pub limit: u32,
    /// Elements (or candidate nodes) that were not emitted because of the stop.
    pub dropped: u32,
    pub reason: TruncReason,
    /// Nodes the search visited before it stopped, for visit-capped searches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visited: Option<u32>,
}

/// The impact of one changed symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolImpact {
    pub seed: SymbolKey,
    pub seed_id: String,
    /// The graph the seed was resolved on: base for removed symbols, head otherwise.
    pub side: GraphSide,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<SeedSkip>,
    /// No included, non-mocked test with score ≥ 0.8 covers the seed (IMP-005).
    pub untested: bool,
    pub elements: Vec<ImpactElement>,
    pub truncation: Vec<Truncation>,
}

impl SymbolImpact {
    /// The elements of one relation, in sort order.
    pub fn of(&self, relation: Relation) -> impl Iterator<Item = &ImpactElement> {
        self.elements
            .iter()
            .filter(move |element| element.relation == relation)
    }
}

/// A truncation attributed to its seed, for [`ImpactStats`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeedTruncation {
    pub seed: SymbolKey,
    pub truncation: Truncation,
}

/// Aggregate counters over the whole impact graph (IMP-007).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactStats {
    pub seeds: u32,
    pub total_elements: u32,
    pub elements_by_relation: BTreeMap<Relation, u32>,
    pub truncations: Vec<SeedTruncation>,
    pub seeds_without_graph: u32,
    pub weak_elements: u32,
    pub transitive_pass_ran: bool,
    pub endpoint_search_visits: u32,
}

/// Capability flags: which inputs were available when the graph was built.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactFlags {
    /// No test-case nodes were present; tests were mapped from naming, path and imports only.
    pub test_mapping_degraded: bool,
    /// Resource nodes (tables, queues, env vars, config, external APIs) exist in the graph.
    pub resource_facts_available: bool,
    /// No base graph was supplied, so removed callees and removed-symbol callers are missing.
    pub base_graph_missing: bool,
}

/// The reverse mapping of a changed test file: the production symbols it targets (IMP-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestTargets {
    pub test_path: String,
    pub targets: Vec<SymbolKey>,
}

/// The impact of a whole pull request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactGraph {
    pub schema_version: u16,
    pub symbols: Vec<SymbolImpact>,
    pub budget: ImpactBudget,
    pub stats: ImpactStats,
    pub flags: ImpactFlags,
    #[serde(default)]
    pub test_targets: Vec<TestTargets>,
    /// `blake3(change input hash ‖ head snapshot ‖ base snapshot ‖ budget ‖ IMPACT_VERSION)`,
    /// 64 lowercase hex characters.
    pub input_hash: String,
}

impl ImpactGraph {
    /// The impact of one seed.
    pub fn symbol(&self, seed: SymbolKey) -> Option<&SymbolImpact> {
        self.symbols.iter().find(|impact| impact.seed == seed)
    }

    /// Every element node across all seeds, deduplicated and sorted.
    pub fn element_nodes(&self) -> Vec<NodeKey> {
        let mut nodes: Vec<NodeKey> = self
            .symbols
            .iter()
            .flat_map(|impact| impact.elements.iter().map(|element| element.node))
            .collect();
        nodes.sort();
        nodes.dedup();
        nodes
    }

    /// The graph's JSON Schema, published with the contracts.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(ImpactGraph)
    }
}

/// Sorts elements by `(relation, distance, -min_confidence, node_id)`.
pub fn sort_elements(elements: &mut [ImpactElement]) {
    elements.sort_by(|a, b| {
        a.relation
            .cmp(&b.relation)
            .then_with(|| a.distance.cmp(&b.distance))
            .then_with(|| b.min_confidence.cmp(&a.min_confidence))
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
}

/// `blake3(change ‖ head ‖ base ‖ budget ‖ IMPACT_VERSION)` as 64 hex characters. Every field
/// is length-prefixed so adjacent fields cannot run into each other.
pub fn compute_input_hash(
    change_input_hash: &str,
    head_snapshot: &str,
    base_snapshot: &str,
    budget: &ImpactBudget,
) -> String {
    let mut hasher = blake3::Hasher::new();
    let budget_json = serde_json::to_vec(budget).unwrap_or_default();
    for part in [
        change_input_hash.as_bytes(),
        head_snapshot.as_bytes(),
        base_snapshot.as_bytes(),
        budget_json.as_slice(),
    ] {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.update(&IMPACT_VERSION.to_le_bytes());
    hasher.finalize().to_hex().to_string()
}
