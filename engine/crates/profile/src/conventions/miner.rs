//! The `ConventionMiner` trait and the engine that turns a miner's samples into a
//! [`Convention`] (PROF-003).

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use codegraph::NodeKey;
use globset::{Glob, GlobSet, GlobSetBuilder};
use review_core::version::ProfileVersion;

use super::confidence::{self, is_enforceable};
use super::scope::narrow_scope;
use crate::infer::architecture::{round3, ModuleIndex};
use crate::infer::roles::RoleIndex;
use crate::infer::ProfileInputs;
use crate::model::{
    Convention, ConventionEvidence, ConventionException, ConventionSource, ExceptionReason,
    RepositoryProfile,
};

/// Most violating samples kept as evidence on a convention.
pub const MAX_VIOLATION_EXAMPLES: usize = 20;

/// One member of a miner's population.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub path: String,
    /// 1-based line of the sample (the symbol's first line, or the fact's).
    pub line: u32,
    /// The symbol's node id, when the sample is a symbol.
    pub symbol: Option<String>,
    pub key: Option<NodeKey>,
    /// Miner-specific data carried from `population` to `conforms` (a thrown class name, a
    /// job id expression, ...).
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conformance {
    Conforms,
    Violates { evidence: String },
    NotApplicable,
}

/// Read-only context every miner gets.
pub struct MiningContext<'a> {
    pub inputs: ProfileInputs<'a>,
    pub roles: &'a RoleIndex,
    pub modules: &'a ModuleIndex,
    generated: GlobSet,
    exceptions: GlobSet,
}

impl std::fmt::Debug for MiningContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MiningContext")
            .field("inputs", &self.inputs)
            .finish_non_exhaustive()
    }
}

fn glob_set(globs: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        if let Ok(glob) = Glob::new(glob) {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

impl<'a> MiningContext<'a> {
    pub fn new(inputs: ProfileInputs<'a>, roles: &'a RoleIndex, modules: &'a ModuleIndex) -> Self {
        let config = inputs.config;
        let mut generated = config.generated.include.clone();
        generated.extend(config.review.generated.ignore.iter().cloned());
        Self {
            inputs,
            roles,
            modules,
            generated: glob_set(&generated),
            exceptions: glob_set(&config.conventions.exceptions),
        }
    }

    /// Why a violating sample is an exception rather than a violation, if it is one.
    pub fn exception_reason(&self, convention_id: &str, sample: &Sample) -> Option<ExceptionReason> {
        if self
            .inputs
            .facts
            .is_waived(convention_id, &sample.path, sample.line)
        {
            Some(ExceptionReason::Annotation)
        } else if self.generated.is_match(&sample.path) {
            Some(ExceptionReason::Generated)
        } else if self.exceptions.is_match(&sample.path) {
            Some(ExceptionReason::Config)
        } else {
            None
        }
    }
}

/// A convention miner. Implementations are pure over the context and deterministic.
pub trait ConventionMiner: Send + Sync {
    /// The convention id (`controllers_do_not_access_repositories`, ...).
    fn id(&self) -> &str;
    /// Bumped whenever the miner's semantics change; it enters `profile_version`.
    fn version(&self) -> u32;
    /// The convention as a sentence.
    fn rule(&self) -> String;
    /// Every sample the convention could apply to, in a deterministic order.
    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample>;
    fn conforms(&self, sample: &Sample, cx: &MiningContext<'_>) -> Conformance;
    /// Fills profile sections the miner owns (testing, API, queues, ...). Runs after mining.
    fn contribute(&self, _cx: &MiningContext<'_>, _profile: &mut RepositoryProfile) {}
}

/// Samples classified by a miner.
#[derive(Debug, Clone, Default)]
pub struct Classified {
    pub conforming: Vec<Sample>,
    pub violating: Vec<(Sample, String)>,
    pub exceptions: Vec<(Sample, ExceptionReason)>,
}

pub fn classify(miner: &dyn ConventionMiner, cx: &MiningContext<'_>) -> Classified {
    let mut out = Classified::default();
    for sample in miner.population(cx) {
        match miner.conforms(&sample, cx) {
            Conformance::Conforms => out.conforming.push(sample),
            Conformance::NotApplicable => {}
            Conformance::Violates { evidence } => match cx.exception_reason(miner.id(), &sample) {
                Some(reason) => out.exceptions.push((sample, reason)),
                None => out.violating.push((sample, evidence)),
            },
        }
    }
    out
}

/// Runs one miner. `None` when its population is empty (no convention is emitted).
pub fn mine(
    miner: &dyn ConventionMiner,
    cx: &MiningContext<'_>,
    profile_version: ProfileVersion,
    computed_at: DateTime<Utc>,
) -> Option<Convention> {
    let classified = classify(miner, cx);
    from_classified(miner, cx, &classified, profile_version, computed_at)
}

pub fn from_classified(
    miner: &dyn ConventionMiner,
    cx: &MiningContext<'_>,
    classified: &Classified,
    profile_version: ProfileVersion,
    computed_at: DateTime<Utc>,
) -> Option<Convention> {
    if classified.conforming.is_empty() && classified.violating.is_empty() {
        return None;
    }
    let judged: Vec<(&str, bool)> = classified
        .conforming
        .iter()
        .map(|s| (s.path.as_str(), true))
        .chain(classified.violating.iter().map(|(s, _)| (s.path.as_str(), false)))
        .collect();
    let scope = narrow_scope(&judged);
    let samples = scope.conforming + scope.violating;
    let modules: BTreeSet<String> = classified
        .conforming
        .iter()
        .map(|s| &s.path)
        .chain(classified.violating.iter().map(|(s, _)| &s.path))
        .filter(|p| scope.contains(p))
        .map(|p| cx.modules.module_of(p))
        .collect();
    let consistency = confidence::consistency(scope.conforming, scope.violating);
    let conf = confidence::confidence(
        consistency,
        samples,
        u32::try_from(modules.len()).unwrap_or(u32::MAX),
    );
    let confidence = round3(conf as f32);
    let violation_examples = classified
        .violating
        .iter()
        .filter(|(s, _)| scope.contains(&s.path))
        .take(MAX_VIOLATION_EXAMPLES)
        .map(|(s, evidence)| ConventionEvidence {
            path: s.path.clone(),
            line: s.line,
            symbol: s.symbol.clone(),
            detail: evidence.clone(),
        })
        .collect();
    let exceptions = classified
        .exceptions
        .iter()
        .map(|(s, reason)| ConventionException {
            path: s.path.clone(),
            symbol: s.symbol.clone(),
            reason: *reason,
        })
        .collect();
    Some(Convention {
        id: miner.id().to_owned(),
        rule: miner.rule(),
        scope: scope.glob,
        samples,
        violations: scope.violating,
        consistency: round3(consistency as f32),
        confidence,
        enforceable: is_enforceable(confidence, samples),
        exceptions,
        violation_examples,
        source: ConventionSource::Inferred,
        miner_version: miner.version(),
        computed_at,
        profile_version,
    })
}
