//! Rule document and convention candidates (CTX-003).
//!
//! Precedence (PRD section 65): policy 1.0, documented 0.8, inferred 0.6 (only with confidence
//! >= 0.9 and at least 10 samples, otherwise excluded), generic 0.3.

use globset::Glob;
use profile::model::{Convention, ConventionSource};
use review_core::location::DiffSide;
use serde_json::json;

use super::{Candidate, CandidateGenerator, CandidatePool, GenStats, SelectionInput};
use crate::model::{CandidateSource, ItemKind};

/// Rule text is cut to this many characters.
pub const MAX_RULE_CHARS: usize = 1_500;
pub const MIN_INFERRED_CONFIDENCE: f32 = 0.9;
pub const MIN_INFERRED_SAMPLES: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSource {
    Policy,
    Documented,
    Inferred,
    Generic,
}

impl RuleSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Documented => "documented",
            Self::Inferred => "inferred",
            Self::Generic => "generic",
        }
    }
}

/// One applicable rule: a policy entry, a documented rule or a convention.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleEntry {
    pub id: String,
    pub text: String,
    /// Glob of the paths the rule applies to.
    pub scope: String,
    pub source: RuleSource,
    pub confidence: f32,
    pub samples: u32,
}

impl RuleEntry {
    pub fn from_convention(c: &Convention) -> Self {
        Self {
            id: c.id.clone(),
            text: c.rule.clone(),
            scope: c.scope.clone(),
            source: match c.source {
                ConventionSource::Explicit => RuleSource::Policy,
                ConventionSource::Documented => RuleSource::Documented,
                ConventionSource::Inferred => RuleSource::Inferred,
            },
            confidence: c.confidence,
            samples: c.samples,
        }
    }

    /// The `rule_priority` signal, or `None` when the rule is excluded.
    pub fn priority(&self) -> Option<f32> {
        match self.source {
            RuleSource::Policy => Some(1.0),
            RuleSource::Documented => Some(0.8),
            RuleSource::Inferred => (self.confidence >= MIN_INFERRED_CONFIDENCE
                && self.samples >= MIN_INFERRED_SAMPLES)
                .then_some(0.6),
            RuleSource::Generic => Some(0.3),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RuleDocCandidates;

impl CandidateGenerator for RuleDocCandidates {
    fn source(&self) -> CandidateSource {
        CandidateSource::RuleDoc
    }

    fn generate(&self, input: &SelectionInput<'_>, out: &mut CandidatePool) -> GenStats {
        let mut stats = GenStats::default();
        if !input.profile_available {
            stats.degraded = true;
            return stats;
        }
        for rule in &input.rules {
            let Some(priority) = rule.priority() else {
                stats.skipped += 1;
                continue;
            };
            let Ok(glob) = Glob::new(&rule.scope) else {
                stats.skipped += 1;
                continue;
            };
            let matcher = glob.compile_matcher();
            if !input
                .seeds
                .iter()
                .any(|s| matcher.is_match(s.file.as_str()))
            {
                stats.skipped += 1;
                continue;
            }
            let kind = match rule.source {
                RuleSource::Policy | RuleSource::Documented => ItemKind::RuleDoc,
                RuleSource::Inferred | RuleSource::Generic => ItemKind::Convention,
            };
            let text: String = rule.text.chars().take(MAX_RULE_CHARS).collect();
            let mut c = Candidate::named(
                rule.id.clone(),
                kind,
                DiffSide::Head,
                CandidateSource::RuleDoc,
            );
            c.distance = Some(1);
            c.relation = Some(rule.source.as_str().to_owned());
            c.raw.rule_priority = priority;
            c.structured = Some(json!({
                "id": rule.id,
                "rule": text,
                "source": rule.source.as_str(),
                "scope": rule.scope,
                "confidence": rule.confidence,
            }));
            stats.record(out.insert(c));
        }
        stats
    }
}
