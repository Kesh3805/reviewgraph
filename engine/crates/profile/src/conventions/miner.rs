//! The `ConventionMiner` trait and the sample vocabulary (PROF-003).

use crate::model::RepositoryProfile;

/// Read access to the graph facts a miner needs. Concrete miners (PROF-004/005) downcast through
/// their own extension traits; the framework only needs it as an opaque `Send + Sync` handle.
pub trait GraphView: Send + Sync {}

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
