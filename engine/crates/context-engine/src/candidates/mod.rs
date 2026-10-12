//! Candidate generation (CTX-002..CTX-004, CTX-008): every generator adds candidates to one
//! [`CandidatePool`], which merges duplicates deterministically.
//!
//! Generators run in a fixed order (structural, tests, config, API, rule docs, lexical; semantic
//! last and only to fill budget) so the pool content is a pure function of the inputs.

pub mod api;
pub mod config;
pub mod lexical;
pub mod rules;
pub mod semantic;
pub mod structural;
pub mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use impact::ImpactGraph;
use review_core::ids::{CommitSha, SymbolKey};
use review_core::location::{DiffSide, RepoPath};
use review_core::reviewer_type::ReviewerType;

use crate::model::{CandidateSource, ItemKind, PathStep};
use crate::ranking::Signals;

/// Raw per-candidate signal inputs, each in `[0, 1]`. Merged by maximum.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RawSignals {
    pub structural: f32,
    pub execution_path: f32,
    pub changed_code_similarity: f32,
    pub test_relationship: f32,
    pub api_relationship: f32,
    pub config_relationship: f32,
    pub semantic_similarity: f32,
    pub risk_importance: f32,
    /// Rule precedence (policy 1.0, documented 0.8, inferred 0.6, generic 0.3); only set on
    /// rule and convention candidates.
    pub rule_priority: f32,
}

impl RawSignals {
    pub fn merge_max(&mut self, other: &Self) {
        self.structural = self.structural.max(other.structural);
        self.execution_path = self.execution_path.max(other.execution_path);
        self.changed_code_similarity = self
            .changed_code_similarity
            .max(other.changed_code_similarity);
        self.test_relationship = self.test_relationship.max(other.test_relationship);
        self.api_relationship = self.api_relationship.max(other.api_relationship);
        self.config_relationship = self.config_relationship.max(other.config_relationship);
        self.semantic_similarity = self.semantic_similarity.max(other.semantic_similarity);
        self.risk_importance = self.risk_importance.max(other.risk_importance);
        self.rule_priority = self.rule_priority.max(other.rule_priority);
    }

    /// The ranking signals for a candidate at `distance`. Rule priority is carried by the
    /// `structural` signal (a rule has no graph relation).
    pub fn to_signals(&self, distance: Option<u8>) -> Signals {
        Signals {
            structural: self.structural.max(self.rule_priority),
            proximity: crate::ranking::proximity(distance),
            execution_path: self.execution_path,
            changed_code_similarity: self.changed_code_similarity,
            test_relationship: self.test_relationship,
            api_relationship: self.api_relationship,
            config_relationship: self.config_relationship,
            semantic_similarity: self.semantic_similarity,
            historical: 0.0,
            risk_importance: self.risk_importance,
        }
        .clamped()
    }
}

/// Identity of a candidate in the pool: `(node, side)`, or `(kind, title)` for items that are
/// not graph nodes (rule docs, config files).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PoolKey {
    Node(SymbolKey, DiffSide),
    Named(ItemKind, String),
}

/// A line range in a file at one side of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSpan {
    pub file: RepoPath,
    pub start_line: u32,
    pub end_line: u32,
    pub side: DiffSide,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub node: Option<SymbolKey>,
    pub title: String,
    pub kind: ItemKind,
    pub side: DiffSide,
    pub sources: BTreeSet<CandidateSource>,
    /// Hops from the nearest seed; `None` for lexical and semantic candidates.
    pub distance: Option<u8>,
    pub path: Vec<PathStep>,
    pub min_confidence: f32,
    pub relation: Option<String>,
    /// Reached only through a low-confidence edge (ranking applies a penalty).
    pub weak: bool,
    pub pinned: bool,
    pub raw: RawSignals,
    pub span: Option<LineSpan>,
    /// Structured payload for non-code items (endpoint attributes, config keys, rule text).
    pub structured: Option<serde_json::Value>,
}

impl Candidate {
    /// A bare candidate for a graph node.
    pub fn node(
        node: SymbolKey,
        title: impl Into<String>,
        kind: ItemKind,
        side: DiffSide,
        source: CandidateSource,
    ) -> Self {
        Self {
            node: Some(node),
            title: title.into(),
            kind,
            side,
            sources: BTreeSet::from([source]),
            distance: None,
            path: Vec::new(),
            min_confidence: 1.0,
            relation: None,
            weak: false,
            pinned: false,
            raw: RawSignals::default(),
            span: None,
            structured: None,
        }
    }

    /// A bare candidate that is not a graph node.
    pub fn named(
        title: impl Into<String>,
        kind: ItemKind,
        side: DiffSide,
        source: CandidateSource,
    ) -> Self {
        let mut c = Self::node(SymbolKey::from_bytes([0; 16]), title, kind, side, source);
        c.node = None;
        c
    }

    pub fn key(&self) -> PoolKey {
        match self.node {
            Some(node) => PoolKey::Node(node, self.side),
            None => PoolKey::Named(self.kind, self.title.clone()),
        }
    }
}

fn kind_priority(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::ChangedSymbol => 0,
        ItemKind::ClassSignature => 1,
        ItemKind::Test => 2,
        ItemKind::ApiEndpoint => 3,
        _ => 4,
    }
}

/// Candidates by identity. Iteration order is the key order, so everything downstream is
/// deterministic.
#[derive(Debug, Clone, Default)]
pub struct CandidatePool {
    by_key: BTreeMap<PoolKey, Candidate>,
}

impl CandidatePool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn get(&self, key: &PoolKey) -> Option<&Candidate> {
        self.by_key.get(key)
    }

    pub fn get_node(&self, node: SymbolKey, side: DiffSide) -> Option<&Candidate> {
        self.by_key.get(&PoolKey::Node(node, side))
    }

    pub fn get_node_mut(&mut self, node: SymbolKey, side: DiffSide) -> Option<&mut Candidate> {
        self.by_key.get_mut(&PoolKey::Node(node, side))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Candidate> {
        self.by_key.values()
    }

    pub fn into_candidates(self) -> Vec<Candidate> {
        self.by_key.into_values().collect()
    }

    /// Inserts or merges. Returns `true` when the candidate was new.
    ///
    /// Merge rule: sources are unioned, distance is the minimum, the path with the higher
    /// `min_confidence` is kept (ties keep the existing one), raw signals merge by maximum.
    /// A merged candidate is weak only if every offer was weak.
    pub fn insert(&mut self, cand: Candidate) -> bool {
        let key = cand.key();
        let Some(existing) = self.by_key.get_mut(&key) else {
            self.by_key.insert(key, cand);
            return true;
        };
        existing.sources.extend(cand.sources.iter().copied());
        existing.distance = match (existing.distance, cand.distance) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        existing.raw.merge_max(&cand.raw);
        existing.pinned |= cand.pinned;
        existing.weak = existing.weak && cand.weak;
        if kind_priority(cand.kind) < kind_priority(existing.kind) {
            existing.kind = cand.kind;
        }
        if cand.min_confidence > existing.min_confidence
            || (existing.path.is_empty() && !cand.path.is_empty())
        {
            existing.min_confidence = cand.min_confidence;
            existing.path = cand.path;
            existing.relation = cand.relation.or_else(|| existing.relation.take());
        } else if existing.relation.is_none() {
            existing.relation = cand.relation;
        }
        if existing.span.is_none() {
            existing.span = cand.span;
        }
        if existing.structured.is_none() {
            existing.structured = cand.structured;
        }
        false
    }
}

/// What a generator did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GenStats {
    pub added: u32,
    pub merged: u32,
    pub skipped: u32,
    /// The generator's input was unavailable (for example, no impact graph).
    pub degraded: bool,
}

impl GenStats {
    pub fn record(&mut self, inserted: bool) {
        if inserted {
            self.added += 1;
        } else {
            self.merged += 1;
        }
    }
}

/// Where a graph node lives and what it is called (the graph lookup is injected by the
/// composition root; this crate performs no I/O).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    pub title: String,
    pub file: RepoPath,
    pub start_line: u32,
    pub end_line: u32,
}

pub trait NodeLocator {
    fn locate(&self, node: SymbolKey, side: DiffSide) -> Option<NodeInfo>;
}

/// The container (class) of a changed member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedContainer {
    pub key: SymbolKey,
    pub title: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// A changed symbol of the cluster, reduced to what context selection reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangedSeed {
    pub key: SymbolKey,
    pub title: String,
    pub file: RepoPath,
    /// The side of `start_line..=end_line`: `Base` for removed symbols, `Head` otherwise.
    pub side: DiffSide,
    pub start_line: u32,
    pub end_line: u32,
    /// The body changed (a modified symbol contributes both sides).
    pub body_changed: bool,
    /// Base-side range of a modified symbol; the head range is reused when unknown.
    pub base_range: Option<(u32, u32)>,
    pub removed: bool,
    pub is_test: bool,
    /// Formatting-only or generated: no context is produced for it.
    pub skipped: bool,
    /// Per-symbol risk contribution in `[0, 1]` (RISK-004).
    pub risk: f32,
    pub container: Option<SeedContainer>,
}

impl ChangedSeed {
    /// Maps the impact crate's symbol input onto a seed.
    pub fn from_symbol_input(input: &impact::input::SymbolInput, risk: f32) -> Self {
        use review_core::change::SymbolChange;
        let s = &input.symbol;
        let (body_changed, removed) = match &s.change {
            SymbolChange::Added => (false, false),
            SymbolChange::Removed => (false, true),
            SymbolChange::Modified { body, .. } => (*body, false),
            SymbolChange::Renamed { .. } => (input.rename_body_changed, false),
        };
        Self {
            key: s.symbol_key,
            title: s.symbol_id.as_str().to_owned(),
            file: s.path.clone(),
            side: if removed { DiffSide::Base } else { s.side },
            start_line: s.range.start.line,
            end_line: s.range.end.line,
            body_changed,
            base_range: None,
            removed,
            is_test: input.test,
            skipped: input.is_skipped(),
            risk,
            container: None,
        }
    }
}

/// Everything candidate generation reads for one `(cluster, reviewer)`.
pub struct SelectionInput<'a> {
    pub reviewer: ReviewerType,
    pub head_commit: CommitSha,
    pub base_commit: CommitSha,
    pub seeds: Vec<ChangedSeed>,
    /// `None` when the impact graph is unavailable (structural context degrades to seeds).
    pub impact: Option<&'a ImpactGraph>,
    pub locator: Option<&'a dyn NodeLocator>,
    /// Per-node risk contribution for nodes other than the seeds.
    pub node_risk: BTreeMap<SymbolKey, f32>,
    /// Nodes carrying authorization, validation or authentication signals (security filter).
    pub trust_nodes: BTreeSet<SymbolKey>,
    /// Configuration files changed in the cluster, with their keys (CTX-003).
    pub config_files: Vec<config::ConfigFile>,
    /// API contract facts of the cluster (CTX-003).
    pub api_changes: Vec<api::ApiChange>,
    /// Policy, documented and inferred rules of the repository profile (CTX-003).
    pub rules: Vec<rules::RuleEntry>,
    /// `false` when no repository profile exists; recorded in provenance, no rule candidates.
    pub profile_available: bool,
}

impl SelectionInput<'_> {
    /// An input with no seeds and no optional facts.
    pub fn new(reviewer: ReviewerType, head_commit: CommitSha, base_commit: CommitSha) -> Self {
        Self {
            reviewer,
            head_commit,
            base_commit,
            seeds: Vec::new(),
            impact: None,
            locator: None,
            node_risk: BTreeMap::new(),
            trust_nodes: BTreeSet::new(),
            config_files: Vec::new(),
            api_changes: Vec::new(),
            rules: Vec::new(),
            profile_available: true,
        }
    }
}

impl fmt::Debug for SelectionInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectionInput")
            .field("reviewer", &self.reviewer)
            .field("seeds", &self.seeds.len())
            .field("impact", &self.impact.is_some())
            .finish_non_exhaustive()
    }
}

impl SelectionInput<'_> {
    pub fn seed_keys(&self) -> BTreeSet<SymbolKey> {
        self.seeds.iter().map(|s| s.key).collect()
    }

    pub fn commit_for(&self, side: DiffSide) -> &CommitSha {
        match side {
            DiffSide::Base => &self.base_commit,
            DiffSide::Head => &self.head_commit,
        }
    }
}

/// One candidate generator.
pub trait CandidateGenerator {
    fn source(&self) -> CandidateSource;
    fn generate(&self, input: &SelectionInput<'_>, out: &mut CandidatePool) -> GenStats;
}

/// Converts an impact path into the model's path steps.
pub(crate) fn convert_path(path: &[impact::graph::PathStep]) -> Vec<PathStep> {
    path.iter()
        .map(|s| PathStep {
            relation: s.edge.to_string(),
            target: s.to.to_string(),
            confidence: s.confidence.as_f32(),
        })
        .collect()
}
