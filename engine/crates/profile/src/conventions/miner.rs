//! The `ConventionMiner` trait and the sample vocabulary (PROF-003).

use std::collections::BTreeMap;

use analysis_ir::facts::SyntaxFact;
use codegraph::query::GraphQuery;

use crate::model::RepositoryProfile;

/// Read access to the graph facts a miner needs: the immutable graph (through the shared
/// [`GraphQuery`] interface, so a PR-head overlay works too) and the per-symbol syntax facts the
/// analyzer extracted (database writes, transaction wrappers, throws, ...).
pub trait GraphView: Send + Sync {
    /// The graph being mined.
    fn graph(&self) -> &dyn GraphQuery;

    /// Syntax facts of the symbol with this canonical node id, in source order. Empty when the
    /// symbol has none or the facts were not loaded.
    fn facts(&self, _node_id: &str) -> &[SyntaxFact] {
        &[]
    }
}

/// A [`GraphView`] over any [`GraphQuery`], with syntax facts keyed by canonical node id.
pub struct CodeGraphView<'g> {
    graph: &'g dyn GraphQuery,
    facts: BTreeMap<String, Vec<SyntaxFact>>,
}

impl std::fmt::Debug for CodeGraphView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeGraphView")
            .field("nodes", &self.graph.node_count())
            .field("fact_symbols", &self.facts.len())
            .finish()
    }
}

impl<'g> CodeGraphView<'g> {
    pub fn new(graph: &'g dyn GraphQuery) -> Self {
        Self {
            graph,
            facts: BTreeMap::new(),
        }
    }

    /// Attaches the syntax facts of one symbol.
    #[must_use]
    pub fn with_facts(mut self, node_id: impl Into<String>, facts: Vec<SyntaxFact>) -> Self {
        self.facts.insert(node_id.into(), facts);
        self
    }
}

impl GraphView for CodeGraphView<'_> {
    fn graph(&self) -> &dyn GraphQuery {
        self.graph
    }

    fn facts(&self, node_id: &str) -> &[SyntaxFact] {
        self.facts.get(node_id).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// The profile being built; miners may read layers/modules already inferred (PROF-002).
pub type PartialProfile = RepositoryProfile;

/// One member of a miner's population.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub id: String,
    pub path: String,
    pub module: String,
    /// `// review-ignore: <convention-id>` annotations found on the sample.
    pub ignore_annotations: Vec<String>,
    /// Miner-private payload (for example a verdict computed while collecting the population).
    pub meta: String,
}

impl Sample {
    /// A sample with no annotations and no payload; the module is derived from the path.
    pub fn new(id: impl Into<String>, path: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            id: id.into(),
            module: crate::infer::module_of(&path),
            path,
            ignore_annotations: Vec::new(),
            meta: String::new(),
        }
    }

    #[must_use]
    pub fn with_meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = meta.into();
        self
    }
}

/// How one sample relates to the convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conformance {
    Conforms,
    Violates { evidence: String },
    NotApplicable,
}

pub trait ConventionMiner: Send + Sync {
    fn id(&self) -> &'static str;
    /// Bumped whenever the miner's semantics change; feeds `profile_version`.
    fn version(&self) -> u32;
    /// Human-readable statement of the rule.
    fn rule(&self) -> &'static str;
    /// The population, for example every service method that writes to the database.
    fn population(&self, g: &dyn GraphView, p: &PartialProfile) -> Vec<Sample>;
    fn conforms(&self, s: &Sample, g: &dyn GraphView) -> Conformance;
}
