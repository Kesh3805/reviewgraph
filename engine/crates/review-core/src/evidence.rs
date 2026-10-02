//! Typed evidence (DOM-007, PRD §52).
//!
//! Evidence is machine-readable so verification can check it: symbol keys, source ranges and
//! the relations a reviewer claims. A published finding needs at least one *effectively*
//! strong item ([`has_strong_evidence`]); model-claimed strength is never trusted on its own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::ids::{SymbolId, SymbolKey};
use crate::location::SourceLocation;
use crate::reviewer_type::ReviewerType;

const MAX_CLAIM_CHARS: usize = 500;
const MAX_DISPLAY_CHARS: usize = 200;
const MAX_VIA_HOPS: usize = 8;

/// The twelve PRD §52 evidence kinds.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    ChangedSource,
    CallerPath,
    CalleePath,
    TestBehavior,
    InterfaceContract,
    Configuration,
    DatabaseSchema,
    RepositoryConvention,
    CompilerDiagnostic,
    LintResult,
    StaticAnalysisResult,
    HistoricalRegression,
}

impl EvidenceKind {
    pub const ALL: [EvidenceKind; 12] = [
        Self::ChangedSource,
        Self::CallerPath,
        Self::CalleePath,
        Self::TestBehavior,
        Self::InterfaceContract,
        Self::Configuration,
        Self::DatabaseSchema,
        Self::RepositoryConvention,
        Self::CompilerDiagnostic,
        Self::LintResult,
        Self::StaticAnalysisResult,
        Self::HistoricalRegression,
    ];

    /// The strongest an item of this kind can ever be. Conventions and history are not proof on
    /// their own (the R10 anti-reinforcement stance), so they top out at `Supporting`.
    pub const fn max_strength(self) -> EvidenceStrength {
        match self {
            Self::RepositoryConvention | Self::HistoricalRegression => EvidenceStrength::Supporting,
            _ => EvidenceStrength::Strong,
        }
    }
}

/// Ordered ascending: `Weak < Supporting < Strong`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    Weak,
    Supporting,
    Strong,
}

/// Who produced an evidence item.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceOrigin {
    /// Claimed by a model reviewer.
    Reviewer { reviewer: ReviewerType },
    /// Produced by a deterministic tool. `tool` is a tool name only, never a command line.
    Deterministic { tool: String },
    /// Read from the code graph.
    Graph,
    /// Gathered by verification stage `stage`.
    Verification { stage: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EvidenceVerification {
    Unverified,
    Confirmed { stage: u8 },
    Refuted { stage: u8, reason: String },
}

/// A reference to a symbol with a short human-readable label (at most 200 characters).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolRef {
    pub id: SymbolId,
    pub key: SymbolKey,
    pub display: String,
}

impl SymbolRef {
    /// Derives the key from the id and validates the display length.
    pub fn new(id: SymbolId, display: impl Into<String>) -> Result<Self, CoreError> {
        let key = SymbolKey::of(&id);
        Self {
            id,
            key,
            display: display.into(),
        }
        .validated()
    }

    fn validated(self) -> Result<Self, CoreError> {
        check_chars("symbol_ref.display", &self.display, 0, MAX_DISPLAY_CHARS)?;
        Ok(self)
    }
}

impl<'de> Deserialize<'de> for SymbolRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            id: SymbolId,
            key: SymbolKey,
            display: String,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            id: r.id,
            key: r.key,
            display: r.display,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

/// A relation a reviewer claims between two symbols. Verification (VER-003) maps each variant
/// onto code-graph edge kinds; `review-core` does not depend on `codegraph`, so the mapping is
/// documented here and implemented there:
///
/// | Variant | Graph edge(s) |
/// |---|---|
/// | `Calls` | `CALLS` (direct) |
/// | `Reaches` | a path of `CALLS` edges |
/// | `Tests` | `TESTS` |
/// | `Implements` | `IMPLEMENTS` |
/// | `Extends` | `EXTENDS` |
/// | `Overrides` | `OVERRIDES` |
/// | `DependsOn` | `IMPORTS` / `DEPENDS_ON` |
/// | `ReadsConfig` | `READS_CONFIG` |
/// | `ReadsTable` | `READS_TABLE` |
/// | `WritesTable` | `WRITES_TABLE` |
/// | `ProducesJob` | `PRODUCES_JOB` |
/// | `ConsumesJob` | `CONSUMES_JOB` |
/// | `HandlesRoute` | `HANDLED_BY` / `ROUTES_TO` |
/// | `GuardedBy` | `GUARDED_BY` |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimedRelation {
    Calls,
    Reaches,
    Tests,
    Implements,
    Extends,
    Overrides,
    DependsOn,
    ReadsConfig,
    ReadsTable,
    WritesTable,
    ProducesJob,
    ConsumesJob,
    HandlesRoute,
    GuardedBy,
}

/// `from --relation--> to`, optionally through `via` (the claimed path, at most 8 hops).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationClaim {
    pub from: SymbolRef,
    pub to: SymbolRef,
    pub relation: ClaimedRelation,
    pub via: Vec<SymbolRef>,
}

impl RelationClaim {
    pub fn new(
        from: SymbolRef,
        to: SymbolRef,
        relation: ClaimedRelation,
        via: Vec<SymbolRef>,
    ) -> Result<Self, CoreError> {
        Self {
            from,
            to,
            relation,
            via,
        }
        .validated()
    }

    fn validated(self) -> Result<Self, CoreError> {
        if self.via.len() > MAX_VIA_HOPS {
            return Err(CoreError::OutOfRange {
                field: "relation_claim.via",
                value: format!("{} hops (at most {MAX_VIA_HOPS})", self.via.len()),
            });
        }
        Ok(self)
    }
}

impl<'de> Deserialize<'de> for RelationClaim {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            from: SymbolRef,
            to: SymbolRef,
            relation: ClaimedRelation,
            via: Vec<SymbolRef>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            from: r.from,
            to: r.to,
            relation: r.relation,
            via: r.via,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

/// One piece of evidence. Build the struct, then call [`Evidence::validated`]; deserialization
/// does this automatically.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub kind: EvidenceKind,
    /// What the producer claims. Use [`Evidence::effective_strength`] for decisions.
    pub claimed_strength: EvidenceStrength,
    pub origin: EvidenceOrigin,
    pub verification: EvidenceVerification,
    /// 1 to 500 characters of model- or tool-authored text; treated as untrusted when rendered.
    pub claim: String,
    pub location: Option<SourceLocation>,
    pub symbols: Vec<SymbolRef>,
    pub relation: Option<RelationClaim>,
}

impl Evidence {
    /// Checks: `claim` is 1 to 500 characters; caller/callee paths carry a `relation`; changed
    /// source carries a `location`; the relation's `via` has at most 8 hops.
    pub fn validated(self) -> Result<Self, CoreError> {
        check_chars("evidence.claim", &self.claim, 1, MAX_CLAIM_CHARS)?;
        if matches!(
            self.kind,
            EvidenceKind::CallerPath | EvidenceKind::CalleePath
        ) && self.relation.is_none()
        {
            return Err(CoreError::InvalidId {
                kind: "Evidence",
                reason: format!("{:?} evidence requires a relation", self.kind),
            });
        }
        if self.kind == EvidenceKind::ChangedSource && self.location.is_none() {
            return Err(CoreError::InvalidId {
                kind: "Evidence",
                reason: "ChangedSource evidence requires a location".to_owned(),
            });
        }
        if let Some(rel) = &self.relation {
            if rel.via.len() > MAX_VIA_HOPS {
                return Err(CoreError::OutOfRange {
                    field: "relation_claim.via",
                    value: format!("{} hops (at most {MAX_VIA_HOPS})", rel.via.len()),
                });
            }
        }
        Ok(self)
    }

    /// The strength verification may rely on:
    /// 1. `Refuted` is `Weak`.
    /// 2. Otherwise the claim is capped at the kind's maximum.
    /// 3. A `Reviewer`-origin item that is still `Unverified` is further capped at `Supporting`:
    ///    model-claimed evidence is never strong until verification confirms it.
    /// 4. `Deterministic`, `Graph` and `Verification` origins keep their capped claimed strength.
    pub fn effective_strength(&self) -> EvidenceStrength {
        if matches!(self.verification, EvidenceVerification::Refuted { .. }) {
            return EvidenceStrength::Weak;
        }
        let mut strength = self.claimed_strength.min(self.kind.max_strength());
        if matches!(self.origin, EvidenceOrigin::Reviewer { .. })
            && matches!(self.verification, EvidenceVerification::Unverified)
        {
            strength = strength.min(EvidenceStrength::Supporting);
        }
        strength
    }
}

impl<'de> Deserialize<'de> for Evidence {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            kind: EvidenceKind,
            claimed_strength: EvidenceStrength,
            origin: EvidenceOrigin,
            verification: EvidenceVerification,
            claim: String,
            location: Option<SourceLocation>,
            symbols: Vec<SymbolRef>,
            relation: Option<RelationClaim>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            kind: r.kind,
            claimed_strength: r.claimed_strength,
            origin: r.origin,
            verification: r.verification,
            claim: r.claim,
            location: r.location,
            symbols: r.symbols,
            relation: r.relation,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

/// Whether any item is *effectively* strong (PRD §52: every published finding needs at least
/// one strong evidence source). Claimed strength alone never counts.
pub fn has_strong_evidence(evidence: &[Evidence]) -> bool {
    evidence
        .iter()
        .any(|e| e.effective_strength() == EvidenceStrength::Strong)
}

fn check_chars(field: &'static str, s: &str, min: usize, max: usize) -> Result<(), CoreError> {
    let n = s.chars().count();
    if n < min || n > max {
        return Err(CoreError::OutOfRange {
            field,
            value: format!("{n} characters (allowed {min}..={max})"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{DiffSide, LineRange, RepoPath};
    use proptest::prelude::*;

    fn sym(name: &str) -> SymbolRef {
        SymbolRef::new(
            SymbolId::from_canonical_unchecked(format!("ts:src/a#{name}/function")),
            name,
        )
        .unwrap()
    }

    fn location() -> SourceLocation {
        SourceLocation {
            path: RepoPath::new("src/a.ts").unwrap(),
            side: DiffSide::Head,
            lines: LineRange::new(1, 2).unwrap(),
            range: None,
        }
    }

    fn relation() -> RelationClaim {
        RelationClaim::new(sym("a"), sym("b"), ClaimedRelation::Calls, vec![]).unwrap()
    }

    fn ev(
        kind: EvidenceKind,
        strength: EvidenceStrength,
        origin: EvidenceOrigin,
        verification: EvidenceVerification,
    ) -> Evidence {
        Evidence {
            kind,
            claimed_strength: strength,
            origin,
            verification,
            claim: "the guard is skipped".to_owned(),
            location: Some(location()),
            symbols: vec![sym("a")],
            relation: Some(relation()),
        }
    }

    fn reviewer() -> EvidenceOrigin {
        EvidenceOrigin::Reviewer {
            reviewer: ReviewerType::Security,
        }
    }

    fn det() -> EvidenceOrigin {
        EvidenceOrigin::Deterministic {
            tool: "eslint".into(),
        }
    }

    #[test]
    fn twelve_evidence_kinds_wire_names() {
        assert_eq!(EvidenceKind::ALL.len(), 12);
        let names: Vec<String> = EvidenceKind::ALL
            .iter()
            .map(|k| {
                serde_json::to_string(k)
                    .unwrap()
                    .trim_matches('"')
                    .to_owned()
            })
            .collect();
        insta::assert_yaml_snapshot!(names);
    }

    #[test]
    fn reviewer_unverified_capped_at_supporting() {
        let e = ev(
            EvidenceKind::ChangedSource,
            EvidenceStrength::Strong,
            reviewer(),
            EvidenceVerification::Unverified,
        );
        assert_eq!(e.effective_strength(), EvidenceStrength::Supporting);
    }

    #[test]
    fn confirmed_reviewer_evidence_can_be_strong() {
        let e = ev(
            EvidenceKind::ChangedSource,
            EvidenceStrength::Strong,
            reviewer(),
            EvidenceVerification::Confirmed { stage: 4 },
        );
        assert_eq!(e.effective_strength(), EvidenceStrength::Strong);
    }

    #[test]
    fn refuted_is_weak() {
        for origin in [reviewer(), det(), EvidenceOrigin::Graph] {
            let e = ev(
                EvidenceKind::LintResult,
                EvidenceStrength::Strong,
                origin,
                EvidenceVerification::Refuted {
                    stage: 3,
                    reason: "no such edge".into(),
                },
            );
            assert_eq!(e.effective_strength(), EvidenceStrength::Weak);
        }
    }

    #[test]
    fn convention_and_history_never_strong() {
        for kind in [
            EvidenceKind::RepositoryConvention,
            EvidenceKind::HistoricalRegression,
        ] {
            for verification in [
                EvidenceVerification::Unverified,
                EvidenceVerification::Confirmed { stage: 5 },
            ] {
                let e = ev(kind, EvidenceStrength::Strong, det(), verification);
                assert_eq!(
                    e.effective_strength(),
                    EvidenceStrength::Supporting,
                    "{kind:?}"
                );
            }
        }
    }

    #[test]
    fn deterministic_lint_result_strong() {
        let e = ev(
            EvidenceKind::LintResult,
            EvidenceStrength::Strong,
            det(),
            EvidenceVerification::Unverified,
        );
        assert_eq!(e.effective_strength(), EvidenceStrength::Strong);
        let graph = ev(
            EvidenceKind::CallerPath,
            EvidenceStrength::Strong,
            EvidenceOrigin::Graph,
            EvidenceVerification::Unverified,
        );
        assert_eq!(graph.effective_strength(), EvidenceStrength::Strong);
    }

    #[test]
    fn has_strong_evidence_requires_effective_not_claimed() {
        let claimed_only = ev(
            EvidenceKind::ChangedSource,
            EvidenceStrength::Strong,
            reviewer(),
            EvidenceVerification::Unverified,
        );
        assert!(!has_strong_evidence(std::slice::from_ref(&claimed_only)));
        assert!(!has_strong_evidence(&[]));
        let strong = ev(
            EvidenceKind::CompilerDiagnostic,
            EvidenceStrength::Strong,
            det(),
            EvidenceVerification::Unverified,
        );
        assert!(has_strong_evidence(&[claimed_only, strong]));
    }

    #[test]
    fn caller_path_requires_relation() {
        for kind in [EvidenceKind::CallerPath, EvidenceKind::CalleePath] {
            let mut e = ev(
                kind,
                EvidenceStrength::Strong,
                det(),
                EvidenceVerification::Unverified,
            );
            e.relation = None;
            assert!(matches!(
                e.clone().validated(),
                Err(CoreError::InvalidId {
                    kind: "Evidence",
                    ..
                })
            ));
            e.relation = Some(relation());
            assert!(e.validated().is_ok());
        }
    }

    #[test]
    fn changed_source_requires_location() {
        let mut e = ev(
            EvidenceKind::ChangedSource,
            EvidenceStrength::Strong,
            det(),
            EvidenceVerification::Unverified,
        );
        e.location = None;
        assert!(matches!(
            e.clone().validated(),
            Err(CoreError::InvalidId {
                kind: "Evidence",
                ..
            })
        ));
        e.location = Some(location());
        assert!(e.validated().is_ok());
    }

    #[test]
    fn via_path_capped_at_8() {
        let hops = |n: usize| (0..n).map(|i| sym(&format!("h{i}"))).collect::<Vec<_>>();
        assert!(RelationClaim::new(sym("a"), sym("b"), ClaimedRelation::Reaches, hops(8)).is_ok());
        assert!(RelationClaim::new(sym("a"), sym("b"), ClaimedRelation::Reaches, hops(9)).is_err());
        let mut json = serde_json::to_value(
            RelationClaim::new(sym("a"), sym("b"), ClaimedRelation::Reaches, hops(8)).unwrap(),
        )
        .unwrap();
        json["via"] = serde_json::to_value(hops(9)).unwrap();
        assert!(serde_json::from_value::<RelationClaim>(json).is_err());
    }

    #[test]
    fn claim_length_bounds() {
        let mk = |claim: String| {
            let mut e = ev(
                EvidenceKind::LintResult,
                EvidenceStrength::Strong,
                det(),
                EvidenceVerification::Unverified,
            );
            e.claim = claim;
            e
        };
        assert!(mk(String::new()).validated().is_err());
        assert!(mk("a".into()).validated().is_ok());
        assert!(mk("a".repeat(500)).validated().is_ok());
        assert!(mk("a".repeat(501)).validated().is_err());
        // The cap counts characters, not bytes.
        assert!(mk("é".repeat(500)).validated().is_ok());
        assert!(SymbolRef::new(
            SymbolId::from_canonical_unchecked("ts:a#f/function"),
            "x".repeat(201)
        )
        .is_err());
        let json = serde_json::to_string(&mk("a".repeat(501))).unwrap();
        assert!(serde_json::from_str::<Evidence>(&json).is_err());
    }

    #[test]
    fn evidence_roundtrips_through_json() {
        let e = ev(
            EvidenceKind::CallerPath,
            EvidenceStrength::Strong,
            reviewer(),
            EvidenceVerification::Confirmed { stage: 3 },
        );
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Evidence>(&json).unwrap(), e);
        assert!(json.contains(r#""origin":{"type":"reviewer","reviewer":"security"}"#));
    }

    fn any_kind() -> impl Strategy<Value = EvidenceKind> {
        prop::sample::select(EvidenceKind::ALL.to_vec())
    }

    fn any_strength() -> impl Strategy<Value = EvidenceStrength> {
        prop::sample::select(vec![
            EvidenceStrength::Weak,
            EvidenceStrength::Supporting,
            EvidenceStrength::Strong,
        ])
    }

    fn any_origin() -> impl Strategy<Value = EvidenceOrigin> {
        prop::sample::select(vec![
            reviewer(),
            det(),
            EvidenceOrigin::Graph,
            EvidenceOrigin::Verification { stage: 4 },
        ])
    }

    fn any_verification() -> impl Strategy<Value = EvidenceVerification> {
        prop::sample::select(vec![
            EvidenceVerification::Unverified,
            EvidenceVerification::Confirmed { stage: 4 },
            EvidenceVerification::Refuted {
                stage: 4,
                reason: "x".into(),
            },
        ])
    }

    proptest! {
        #[test]
        fn effective_strength_never_exceeds_claimed_or_kind_cap(
            kind in any_kind(),
            strength in any_strength(),
            origin in any_origin(),
            verification in any_verification(),
        ) {
            let e = ev(kind, strength, origin, verification);
            let eff = e.effective_strength();
            prop_assert!(eff <= e.claimed_strength);
            prop_assert!(eff <= e.kind.max_strength());
        }
    }
}
