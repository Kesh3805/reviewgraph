//! Rename/move matching primitives (ADR-005, SID-005).
//!
//! The matcher pairs the symbols one snapshot removed with the symbols another added. Three rules
//! are tried in order (identical body, identical signature and name in a different file, token
//! Jaccard similarity), candidate pairs are sorted by a fixed set of tie-breakers and assigned
//! greedily and one-to-one. When two candidates remain indistinguishable the symbol is left
//! unmatched: a wrong pairing is worse than none.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{SymbolId, SymbolKey};
use crate::symbol::{Hash128, ModulePath, ShingleSet, SymbolKind};

// Re-exported so a caller of the matcher has the rule and transition vocabulary in scope.
pub use crate::lineage::{
    AmbiguityNote, AmbiguityReason, LineageRecord, MatchRule, SymbolTransition,
};

/// Thresholds and caps of the matcher. Calibrated in the SID-006 report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatcherConfig {
    /// Minimum body tokens for an exact-body match: below this, bodies are too generic to pair.
    pub min_tokens_exact: u32,
    /// Minimum body tokens for a fuzzy match.
    pub min_tokens_fuzzy: u32,
    /// Minimum token Jaccard similarity for a fuzzy match.
    pub jaccard_min: f32,
    /// Maximum candidate pairs examined per symbol.
    pub max_candidates: usize,
    /// Above this many removed or added symbols the matcher degrades to rule 1.
    pub degrade_threshold: usize,
    /// A winner that beats the runner-up by less than this is reported as ambiguous.
    pub ambiguity_margin: f32,
}

impl Default for MatcherConfig {
    fn default() -> Self {
        Self {
            min_tokens_exact: 6,
            min_tokens_fuzzy: 12,
            jaccard_min: 0.8,
            max_candidates: 64,
            degrade_threshold: 50_000,
            ambiguity_margin: 0.02,
        }
    }
}

/// Everything the matcher needs about one symbol, with no source text: ids, names, hashes and the
/// body sketch. Built from `ParsedUnit` by the incremental crate.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolRef {
    /// Canonical id.
    pub id: SymbolId,
    /// Storage key of the id.
    pub key: SymbolKey,
    /// What kind of declaration it is; a symbol never matches a different kind.
    pub kind: SymbolKind,
    /// Simple name.
    pub name: String,
    /// Qualified name segments, outermost first.
    pub qualified_name: Vec<String>,
    /// Module path of the file the symbol lives in.
    pub module_path: ModulePath,
    /// Id of the enclosing symbol, if any.
    pub parent_id: Option<SymbolId>,
    /// Normalized signature hash (TSA-007).
    pub signature_hash: Hash128,
    /// Normalized body hash (TSA-007).
    pub body_hash: Hash128,
    /// Number of body tokens.
    pub body_token_count: u32,
    /// Bottom-k shingle sketch of the body.
    pub shingles: ShingleSet,
}

/// Whether a kind's `body_hash` folds its members into one placeholder per member (TSA-007).
///
/// A container is matched by a distinct part of rule 1: its folded stream is `{`, one placeholder
/// per member and `}`, so its `body_token_count` is two plus the member count and never comparable
/// with the leaf minimum. See [`rule_match`].
pub const fn is_container(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Module
            | SymbolKind::Namespace
            | SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Enum
    )
}

impl SymbolRef {
    /// Directory of the module path, used as a tie-breaker.
    pub fn directory(&self) -> &str {
        match self.module_path.as_str().rfind('/') {
            Some(index) => &self.module_path.as_str()[..index],
            None => "",
        }
    }

    /// Distance in path segments between this symbol's directory and another symbol's.
    pub fn directory_distance(&self, other: &SymbolRef) -> usize {
        let mut here: Vec<&str> = self
            .directory()
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        let mut there: Vec<&str> = other
            .directory()
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        while !here.is_empty() && !there.is_empty() && here[0] == there[0] {
            here.remove(0);
            there.remove(0);
        }
        here.len() + there.len()
    }

    /// Same qualified name, so the symbol did not change its name.
    pub fn same_name(&self, other: &SymbolRef) -> bool {
        self.qualified_name == other.qualified_name
    }
}

/// One candidate pair with its score, before assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateEdge {
    /// Index into the removed slice.
    pub from: usize,
    /// Index into the added slice.
    pub to: usize,
    /// Which rule produced the pair.
    pub rule: MatchRule,
    /// Confidence in `[0, 1]`.
    pub similarity: f32,
    /// Both parents were paired, so this member pair is inside matched containers.
    pub same_parent_match: bool,
    /// The two symbols have the same simple name.
    pub same_name: bool,
    /// Path-segment distance between the two directories.
    pub directory_distance: usize,
    /// The pair came from a file-rename hint (a DIFF fact, never a match on its own).
    pub from_rename_hint: bool,
}

impl CandidateEdge {
    /// The documented sort order: best first, with stable tie-breakers so two runs over the same
    /// inputs assign identically.
    pub fn ordering(&self) -> CandidateOrderingKey {
        CandidateOrderingKey {
            similarity: similarity_rank(self.similarity),
            same_parent_match: self.same_parent_match,
            same_name: self.same_name,
            directory_distance: self.directory_distance,
            from: self.from,
            to: self.to,
        }
    }

    /// The semantic tie-breakers, without the index tie-breakers. Two candidates with the same tie
    /// key are equally good, so the symbol is left unmatched rather than guessed.
    pub fn tie_key(&self) -> CandidateTieKey {
        CandidateTieKey {
            similarity: similarity_rank(self.similarity),
            same_parent_match: self.same_parent_match,
            same_name: self.same_name,
            directory_distance: self.directory_distance,
        }
    }
}

/// The part of the ordering that decides whether two candidates are equally good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateTieKey {
    /// Similarity as an integer rank.
    pub similarity: i32,
    /// Whether both parents were matched.
    pub same_parent_match: bool,
    /// Whether the simple names agree.
    pub same_name: bool,
    /// Path-segment distance between the directories.
    pub directory_distance: usize,
}

/// The comparison key of a [`CandidateEdge`], ordered so the best pair compares as `Greater`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateOrderingKey {
    /// Highest similarity first, as the integer rank of the `f32`, so the ordering is total.
    pub similarity: i32,
    /// Prefer a pair whose parents were matched.
    pub same_parent_match: bool,
    /// Then prefer the same simple name.
    pub same_name: bool,
    /// Then prefer the closest directory.
    pub directory_distance: usize,
    /// Then the removed index, which is deterministic because inputs are sorted.
    pub from: usize,
    /// Then the added index.
    pub to: usize,
}

impl Ord for CandidateOrderingKey {
    /// The best candidate compares as `Greater`, so the list can be sorted with `Reverse`.
    fn cmp(&self, other: &Self) -> Ordering {
        self.similarity
            .cmp(&other.similarity)
            .then_with(|| self.same_parent_match.cmp(&other.same_parent_match))
            .then_with(|| self.same_name.cmp(&other.same_name))
            .then_with(|| other.directory_distance.cmp(&self.directory_distance))
            .then_with(|| other.from.cmp(&self.from))
            .then_with(|| other.to.cmp(&self.to))
    }
}

impl PartialOrd for CandidateOrderingKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Maps an `f32` onto a monotonically increasing integer so `Ord` is total.
fn similarity_rank(value: f32) -> i32 {
    if value.is_nan() {
        return i32::MIN;
    }
    (value.clamp(-1.0, 1.0) * 1_000_000.0).round() as i32
}

/// The outcome of one matcher run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MatchResult {
    /// One record per paired symbol, sorted deterministically.
    pub matches: Vec<LineageRecord>,
    /// Added symbols with no counterpart, sorted by key.
    pub unmatched_added: Vec<SymbolKey>,
    /// Removed symbols with no counterpart, sorted by key.
    pub unmatched_removed: Vec<SymbolKey>,
    /// Symbols left unmatched because two candidates were equally plausible.
    pub ambiguous: Vec<AmbiguityNote>,
    /// The run used rule 1 only because the input was over capacity.
    pub degraded: bool,
}

/// The similarity the rule ladder assigns to one candidate pair, if any rule fires.
///
/// Rule 1 (`ExactBody`) is the only rule a container takes part in without a token minimum: a
/// container's body stream is folded into one placeholder per member (TSA-007), so it is two tokens
/// plus the member count and would otherwise never reach `min_tokens_exact`, which would make a
/// renamed class unmatchable (SID-005 requires `ExactBody` on the container hash). The folded stream
/// is still specific, because every placeholder carries the member's kind and name.
pub fn rule_match(
    removed: &SymbolRef,
    added: &SymbolRef,
    cfg: &MatcherConfig,
) -> Option<CandidateEdge> {
    if removed.kind != added.kind {
        return None;
    }
    let jaccard = removed.shingles.jaccard(&added.shingles);
    // Both sides must clear the token minimum: a tiny body on either side is too generic to pair.
    let tokens_exact = removed.body_token_count.min(added.body_token_count);
    let min_tokens_exact = if is_container(removed.kind) {
        0
    } else {
        cfg.min_tokens_exact
    };
    let similarity = if removed.body_hash == added.body_hash && tokens_exact >= min_tokens_exact {
        Some((MatchRule::ExactBody, 1.0_f32))
    } else if removed.signature_hash == added.signature_hash
        && removed.name == added.name
        && removed.module_path != added.module_path
        && tokens_exact >= cfg.min_tokens_fuzzy
    {
        Some((MatchRule::SignatureAndName, jaccard.max(cfg.jaccard_min)))
    } else if jaccard >= cfg.jaccard_min && tokens_exact >= cfg.min_tokens_fuzzy && jaccard > 0.0 {
        Some((MatchRule::TokenSimilarity, jaccard))
    } else {
        None
    };
    let (rule, similarity) = similarity?;
    Some(CandidateEdge {
        from: 0,
        to: 0,
        rule,
        similarity,
        same_parent_match: false,
        same_name: removed.name == added.name,
        directory_distance: removed.directory_distance(added),
        from_rename_hint: false,
    })
}

/// How the identity changed, derived from the two symbols.
pub fn transition_of(removed: &SymbolRef, added: &SymbolRef) -> SymbolTransition {
    let renamed = !removed.same_name(added);
    let moved = removed.module_path != added.module_path;
    match (renamed, moved) {
        (true, true) => SymbolTransition::RenamedAndMoved,
        (true, false) => SymbolTransition::Renamed,
        (false, true) => SymbolTransition::Moved,
        (false, false) => SymbolTransition::Renamed,
    }
}

/// Turns accepted edges into lineage records, marking a record ambiguous when the runner-up of its
/// removed symbol was within [`MatcherConfig::ambiguity_margin`].
pub fn records_from_edges(
    removed: &[SymbolRef],
    added: &[SymbolRef],
    edges: &[CandidateEdge],
    cfg: &MatcherConfig,
) -> MatchResult {
    let mut result = MatchResult::default();
    let mut taken_removed: Vec<bool> = vec![false; removed.len()];
    let mut taken_added: Vec<bool> = vec![false; added.len()];
    let mut ranked: Vec<&CandidateEdge> = edges.iter().collect();
    ranked.sort_by_key(|edge| std::cmp::Reverse(edge.ordering()));

    // An added symbol whose best candidate ties with another is not matched at all.
    let tied = tied_added_indices(added, edges);

    // The runner-up of a removed symbol is the best candidate it did not get, so a record whose
    // winner beat that alternative by less than the margin is flagged. Without this a lone
    // candidate would be compared against itself and every record would look ambiguous.
    let mut runner_up: BTreeMap<usize, f32> = BTreeMap::new();
    for (from, candidates) in grouped_candidates_by_removed(edges) {
        let mut scores: Vec<(i32, f32)> = candidates
            .iter()
            .map(|edge| (similarity_rank(edge.similarity), edge.similarity))
            .collect();
        scores.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        scores.dedup_by_key(|entry| entry.0);
        if let Some((_, second)) = scores.get(1) {
            runner_up.insert(from, *second);
        }
    }

    for edge in ranked {
        if tied.contains(&edge.to) || taken_removed[edge.from] || taken_added[edge.to] {
            continue;
        }
        let (removed_symbol, added_symbol) = (&removed[edge.from], &added[edge.to]);
        let ambiguous = runner_up
            .get(&edge.from)
            .is_some_and(|second| edge.similarity - *second < cfg.ambiguity_margin);
        taken_removed[edge.from] = true;
        taken_added[edge.to] = true;
        result.matches.push(LineageRecord {
            from: removed_symbol.key,
            to: added_symbol.key,
            from_id: removed_symbol.id.clone(),
            to_id: added_symbol.id.clone(),
            transition: transition_of(removed_symbol, added_symbol),
            rule: edge.rule,
            similarity: edge.similarity,
            ambiguous,
        });
    }
    for (index, taken) in taken_added.iter().enumerate() {
        if !taken {
            result.unmatched_added.push(added[index].key);
        }
    }
    for (index, taken) in taken_removed.iter().enumerate() {
        if !taken {
            result.unmatched_removed.push(removed[index].key);
        }
    }
    result.unmatched_added.sort();
    result.unmatched_added.dedup();
    result.unmatched_removed.sort();
    result.unmatched_removed.dedup();
    result.matches.sort_by(|left, right| {
        left.from_id
            .as_str()
            .cmp(right.from_id.as_str())
            .then_with(|| left.to_id.as_str().cmp(right.to_id.as_str()))
    });
    result
}

/// The removed and added pool indices that cannot be paired because two of their candidates are
/// indistinguishable after every semantic tie-breaker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TiedSymbols {
    /// Indices into the removed pool.
    pub removed: BTreeSet<usize>,
    /// Indices into the added pool.
    pub added: BTreeSet<usize>,
}

/// Indices of symbols whose *best* candidates are tied on every semantic tie-breaker. Such a symbol
/// stays unmatched: a wrong pairing is worse than none.
///
/// The check is symmetric. An added symbol with several equally plausible predecessors is ambiguous,
/// and so is a removed symbol with several equally plausible successors; the tie-breakers the sort
/// order documents stop at `same_dir_distance`, so the `from_id`/`to_id` tie-breakers must not be
/// allowed to decide a pairing that nothing else distinguishes. Only the best candidates matter: a
/// symbol whose best candidate is unique is paired with it, however many weaker candidates it has.
pub fn tied_symbol_indices(
    removed: &[SymbolRef],
    added: &[SymbolRef],
    edges: &[CandidateEdge],
) -> TiedSymbols {
    let mut tied = TiedSymbols::default();
    for (to, candidates) in grouped_candidates(edges) {
        if to < added.len() && has_tied_best(&candidates) {
            tied.added.insert(to);
        }
    }
    for (from, candidates) in grouped_candidates_by_removed(edges) {
        if from < removed.len() && has_tied_best(&candidates) {
            tied.removed.insert(from);
        }
    }
    tied
}

/// Whether a symbol's best candidate is indistinguishable from another of its candidates.
fn has_tied_best(candidates: &[&CandidateEdge]) -> bool {
    best_tie_candidates(candidates).is_some()
}

/// The candidates that share the tie key of the best candidate, when more than one of them does.
fn best_tie_candidates<'a>(candidates: &[&'a CandidateEdge]) -> Option<Vec<&'a CandidateEdge>> {
    let best = candidates
        .iter()
        .copied()
        .max_by_key(|edge| edge.ordering())?;
    let tied: Vec<&CandidateEdge> = candidates
        .iter()
        .copied()
        .filter(|edge| edge.tie_key() == best.tie_key())
        .collect();
    if tied.len() > 1 {
        Some(tied)
    } else {
        None
    }
}

/// Indices of added symbols whose best candidates are tied on every semantic tie-breaker. The
/// symmetric form is [`tied_symbol_indices`].
pub fn tied_added_indices(added: &[SymbolRef], edges: &[CandidateEdge]) -> BTreeSet<usize> {
    grouped_candidates(edges)
        .into_iter()
        .filter(|(_, candidates)| has_tied_best(candidates))
        .map(|(to, _)| to)
        .filter(|to| *to < added.len())
        .collect()
}

/// Reports the symbols that stayed unmatched because two candidates were equally plausible.
///
/// A note is emitted for both sides of the ladder: `key` is the symbol that could not be paired
/// (the added symbol when its predecessors tie, the removed symbol when its successors tie) and
/// `candidates` are the equally plausible counterparts, sorted so the report does not depend on the
/// order of the input pools. Reporting is why the caller must pass the full candidate set: the
/// candidates that made a symbol ambiguous are exactly the ones that get filtered out again.
pub fn ambiguity_notes(
    removed: &[SymbolRef],
    added: &[SymbolRef],
    edges: &[CandidateEdge],
) -> Vec<AmbiguityNote> {
    let mut notes: Vec<AmbiguityNote> = Vec::new();
    for (to, candidates) in grouped_candidates(edges) {
        if to >= added.len() {
            continue;
        }
        let Some(tied) = best_tie_candidates(&candidates) else {
            continue;
        };
        notes.push(AmbiguityNote {
            key: added[to].key,
            id: added[to].id.clone(),
            candidates: counterpart_keys(&tied, |edge| {
                removed.get(edge.from).map(|symbol| symbol.key)
            }),
            reason: AmbiguityReason::EqualCandidates,
        });
    }
    for (from, candidates) in grouped_candidates_by_removed(edges) {
        if from >= removed.len() {
            continue;
        }
        let Some(tied) = best_tie_candidates(&candidates) else {
            continue;
        };
        notes.push(AmbiguityNote {
            key: removed[from].key,
            id: removed[from].id.clone(),
            candidates: counterpart_keys(&tied, |edge| added.get(edge.to).map(|symbol| symbol.key)),
            reason: AmbiguityReason::EqualCandidates,
        });
    }
    notes.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.candidates.cmp(&right.candidates))
    });
    notes.dedup();
    notes
}

/// The keys of the tied edges, sorted and deduplicated. An edge whose counterpart index is out of
/// range contributes no key.
fn counterpart_keys(
    edges: &[&CandidateEdge],
    key: impl Fn(&CandidateEdge) -> Option<SymbolKey>,
) -> Vec<SymbolKey> {
    let mut keys: Vec<SymbolKey> = edges.iter().filter_map(|edge| key(edge)).collect();
    keys.sort();
    keys.dedup();
    keys
}

fn grouped_candidates(edges: &[CandidateEdge]) -> BTreeMap<usize, Vec<&CandidateEdge>> {
    let mut by_added: BTreeMap<usize, Vec<&CandidateEdge>> = BTreeMap::new();
    for edge in edges {
        by_added.entry(edge.to).or_default().push(edge);
    }
    by_added
}

fn grouped_candidates_by_removed(edges: &[CandidateEdge]) -> BTreeMap<usize, Vec<&CandidateEdge>> {
    let mut by_removed: BTreeMap<usize, Vec<&CandidateEdge>> = BTreeMap::new();
    for edge in edges {
        by_removed.entry(edge.from).or_default().push(edge);
    }
    by_removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::RepoPath;
    use crate::symbol_id::SymbolIdParts;

    fn hash(seed: u8) -> Hash128 {
        Hash128::of("test", &[seed])
    }

    fn symbol(module: &str, name: &str, kind: SymbolKind, body: u8, tokens: u32) -> SymbolRef {
        let path = RepoPath::new(format!("{module}.ts")).unwrap();
        let id = SymbolIdParts::new(
            "ts",
            crate::symbol_id::module_path_for(&path, crate::language::Language::Typescript),
            vec![name.to_owned()],
            kind,
        )
        .format()
        .unwrap();
        SymbolRef {
            key: SymbolKey::of(&id),
            id,
            kind,
            name: name.to_owned(),
            qualified_name: vec![name.to_owned()],
            module_path: crate::symbol_id::module_path_for(
                &path,
                crate::language::Language::Typescript,
            ),
            parent_id: None,
            signature_hash: hash(200),
            body_hash: hash(body),
            body_token_count: tokens,
            shingles: ShingleSet::of([body as u32, body as u32 + 1]),
        }
    }

    #[test]
    fn rule_one_needs_an_identical_body_and_enough_tokens() {
        let cfg = MatcherConfig::default();
        let a = symbol("src/a", "f", SymbolKind::Function, 1, 10);
        let b = symbol("src/b", "g", SymbolKind::Function, 1, 10);
        let edge = rule_match(&a, &b, &cfg).unwrap();
        assert_eq!(edge.rule, MatchRule::ExactBody);
        assert_eq!(edge.similarity, 1.0);
        let tiny = symbol("src/c", "h", SymbolKind::Function, 1, 3);
        assert!(
            rule_match(&a, &tiny, &cfg).is_none(),
            "tiny bodies never match"
        );
        let other = symbol("src/d", "i", SymbolKind::Function, 2, 10);
        assert!(rule_match(&a, &other, &cfg).is_none());
    }

    #[test]
    fn a_container_pairs_on_its_folded_hash_without_the_leaf_token_minimum() {
        let cfg = MatcherConfig::default();
        // A container's folded body is `{`, one placeholder per member, `}`: four tokens here, far
        // below `min_tokens_exact`, yet the folded stream still names every member.
        let class = symbol("src/a", "OrderService", SymbolKind::Class, 1, 4);
        let renamed = symbol("src/a", "OrdersService", SymbolKind::Class, 1, 4);
        let edge = rule_match(&class, &renamed, &cfg).unwrap();
        assert_eq!(edge.rule, MatchRule::ExactBody);
        assert_eq!(edge.similarity, 1.0);
        let leaf = symbol("src/a", "getId", SymbolKind::Method, 1, 4);
        assert!(
            rule_match(
                &leaf,
                &symbol("src/a", "getName", SymbolKind::Method, 1, 4),
                &cfg
            )
            .is_none(),
            "a four-token leaf is still too generic to pair"
        );
        assert!(is_container(SymbolKind::Class));
        assert!(is_container(SymbolKind::Module));
        assert!(!is_container(SymbolKind::Method));
    }

    #[test]
    fn rule_two_needs_the_same_name_and_a_different_file() {
        let cfg = MatcherConfig::default();
        let a = symbol("src/a", "helper", SymbolKind::Function, 1, 20);
        let moved = symbol("src/other/b", "helper", SymbolKind::Function, 7, 20);
        let edge = rule_match(&a, &moved, &cfg).unwrap();
        assert_eq!(edge.rule, MatchRule::SignatureAndName);
        assert!(edge.similarity >= cfg.jaccard_min);
        assert_eq!(transition_of(&a, &moved), SymbolTransition::Moved);
        let renamed = symbol("src/other/c", "renamed", SymbolKind::Function, 7, 20);
        assert!(
            rule_match(&a, &renamed, &cfg).is_none(),
            "a different name is not rule two"
        );
    }

    #[test]
    fn rule_three_needs_similarity_above_the_threshold() {
        let cfg = MatcherConfig::default();
        let mut a = symbol("src/a", "one", SymbolKind::Function, 1, 20);
        a.shingles = ShingleSet::of([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let mut b = symbol("src/b", "two", SymbolKind::Function, 2, 20);
        b.shingles = ShingleSet::of([1, 2, 3, 4, 5, 6, 7, 8, 9, 99]);
        let edge = rule_match(&a, &b, &cfg).unwrap();
        assert_eq!(edge.rule, MatchRule::TokenSimilarity);
        assert!(edge.similarity >= cfg.jaccard_min);
        let unrelated = symbol("src/c", "three", SymbolKind::Function, 3, 20);
        assert!(rule_match(&a, &unrelated, &cfg).is_none());
    }

    #[test]
    fn kind_mismatch_never_matches() {
        let cfg = MatcherConfig::default();
        let function = symbol("src/a", "x", SymbolKind::Function, 1, 30);
        let method = symbol("src/b", "x", SymbolKind::Method, 1, 30);
        assert!(rule_match(&function, &method, &cfg).is_none());
    }

    #[test]
    fn transitions_cover_the_three_cases() {
        let base = symbol("src/a", "name", SymbolKind::Class, 1, 30);
        assert_eq!(
            transition_of(&base, &symbol("src/a", "other", SymbolKind::Class, 1, 30)),
            SymbolTransition::Renamed
        );
        assert_eq!(
            transition_of(&base, &symbol("src/b", "name", SymbolKind::Class, 1, 30)),
            SymbolTransition::Moved
        );
        assert_eq!(
            transition_of(&base, &symbol("src/b", "other", SymbolKind::Class, 1, 30)),
            SymbolTransition::RenamedAndMoved
        );
    }

    #[test]
    fn ordering_is_total_and_stable() {
        let low = CandidateEdge {
            from: 0,
            to: 0,
            rule: MatchRule::TokenSimilarity,
            similarity: 0.8,
            same_parent_match: false,
            same_name: false,
            directory_distance: 4,
            from_rename_hint: false,
        };
        let mut high = low.clone();
        high.similarity = 0.95;
        assert!(high.ordering() > low.ordering());
        let mut same_name = low.clone();
        same_name.same_name = true;
        assert!(same_name.ordering() > low.ordering());
        let mut near = low.clone();
        near.directory_distance = 1;
        assert!(near.ordering() > low.ordering());
        assert_eq!(low.ordering(), low.clone().ordering());
        let mut later = low.clone();
        later.from = 1;
        assert!(
            later.ordering() < low.ordering(),
            "the index tie-breaker makes the smaller index better"
        );
        assert_eq!(low.tie_key(), low.clone().tie_key());
        assert_eq!(similarity_rank(f32::NAN), i32::MIN);
        assert_eq!(
            similarity_rank(-0.5),
            -500_000,
            "a negative score ranks below zero"
        );
        assert_eq!(similarity_rank(2.0), 1_000_000, "similarity is clamped");
    }

    #[test]
    fn assignment_is_one_to_one_and_reports_ambiguity() {
        let cfg = MatcherConfig::default();
        let removed = vec![
            symbol("src/a", "one", SymbolKind::Function, 1, 30),
            symbol("src/a", "two", SymbolKind::Function, 1, 30),
        ];
        let added = vec![symbol("src/b", "three", SymbolKind::Function, 1, 30)];
        let edges: Vec<CandidateEdge> = removed
            .iter()
            .enumerate()
            .flat_map(|(from, removed_symbol)| {
                added
                    .iter()
                    .enumerate()
                    .filter_map(move |(to, added_symbol)| {
                        rule_match(removed_symbol, added_symbol, &cfg).map(|mut edge| {
                            edge.from = from;
                            edge.to = to;
                            edge
                        })
                    })
            })
            .collect();
        assert_eq!(edges.len(), 2, "both removed symbols are plausible");
        assert_eq!(tied_added_indices(&added, &edges), BTreeSet::from([0]));
        let result = records_from_edges(&removed, &added, &edges, &cfg);
        assert_eq!(
            result.matches.len(),
            0,
            "two equally plausible predecessors must not be paired"
        );
        assert_eq!(result.unmatched_added.len(), 1);
        assert_eq!(result.unmatched_removed.len(), 2);
        let notes = ambiguity_notes(&removed, &added, &edges);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].candidates.len(), 2);
        assert_eq!(notes[0].reason, AmbiguityReason::EqualCandidates);
    }

    #[test]
    fn a_close_runner_up_marks_the_record_ambiguous() {
        let cfg = MatcherConfig::default();
        let removed = vec![symbol("src/a", "one", SymbolKind::Function, 1, 30)];
        let added = vec![
            symbol("src/b", "two", SymbolKind::Function, 1, 30),
            symbol("src/c", "three", SymbolKind::Function, 1, 30),
        ];
        // Two candidates that differ, but by less than the ambiguity margin.
        let edges = vec![
            CandidateEdge {
                from: 0,
                to: 0,
                rule: MatchRule::TokenSimilarity,
                similarity: 0.9,
                same_parent_match: false,
                same_name: false,
                directory_distance: 0,
                from_rename_hint: false,
            },
            CandidateEdge {
                from: 0,
                to: 1,
                rule: MatchRule::TokenSimilarity,
                similarity: 0.89,
                same_parent_match: false,
                same_name: false,
                directory_distance: 0,
                from_rename_hint: false,
            },
        ];
        assert!(tied_added_indices(&added, &edges).is_empty());
        let result = records_from_edges(&removed, &added, &edges, &cfg);
        assert_eq!(result.matches.len(), 1);
        assert!(
            result.matches[0].ambiguous,
            "a winner inside the ambiguity margin must be flagged"
        );
        assert_eq!(result.matches[0].similarity, 0.9);
        assert_eq!(result.unmatched_added.len(), 1);
    }

    #[test]
    fn a_removed_symbol_with_two_indistinguishable_successors_is_tied() {
        let cfg = MatcherConfig::default();
        let removed = vec![symbol("src/a", "helper", SymbolKind::Function, 1, 30)];
        let added = vec![
            symbol("src/b", "one", SymbolKind::Function, 1, 30),
            symbol("src/c", "two", SymbolKind::Function, 1, 30),
        ];
        let edges: Vec<CandidateEdge> = added
            .iter()
            .enumerate()
            .filter_map(|(to, added_symbol)| {
                rule_match(&removed[0], added_symbol, &cfg).map(|mut edge| {
                    edge.to = to;
                    edge
                })
            })
            .collect();
        assert_eq!(edges.len(), 2, "both added symbols are plausible");
        assert!(
            tied_added_indices(&added, &edges).is_empty(),
            "each added symbol has only one candidate"
        );
        assert_eq!(
            tied_symbol_indices(&removed, &added, &edges),
            TiedSymbols {
                removed: BTreeSet::from([0]),
                added: BTreeSet::new(),
            }
        );
        let notes = ambiguity_notes(&removed, &added, &edges);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].key, removed[0].key);
        assert_eq!(notes[0].id, removed[0].id);
        let mut expected = vec![added[0].key, added[1].key];
        expected.sort();
        assert_eq!(
            notes[0].candidates, expected,
            "the equally plausible counterparts are sorted, so the report is input-order free"
        );
    }

    #[test]
    fn a_lone_candidate_is_not_its_own_runner_up() {
        let cfg = MatcherConfig::default();
        // Each removed symbol has exactly one candidate, so no record may be flagged ambiguous.
        let removed = vec![
            symbol("src/a", "one", SymbolKind::Function, 1, 30),
            symbol("src/b", "two", SymbolKind::Function, 2, 30),
        ];
        let added = vec![
            symbol("src/c", "three", SymbolKind::Function, 1, 30),
            symbol("src/d", "four", SymbolKind::Function, 2, 30),
        ];
        let edges: Vec<CandidateEdge> = removed
            .iter()
            .enumerate()
            .flat_map(|(from, removed_symbol)| {
                added
                    .iter()
                    .enumerate()
                    .filter_map(move |(to, added_symbol)| {
                        rule_match(removed_symbol, added_symbol, &cfg).map(|mut edge| {
                            edge.from = from;
                            edge.to = to;
                            edge
                        })
                    })
            })
            .collect();
        assert_eq!(edges.len(), 2);
        let result = records_from_edges(&removed, &added, &edges, &cfg);
        assert_eq!(result.matches.len(), 2);
        assert!(
            result.matches.iter().all(|record| !record.ambiguous),
            "{:?}",
            result.matches
        );
    }
}
