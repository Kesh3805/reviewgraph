//! Candidate generation for the rename matcher (SID-005).
//!
//! Rules are tried in the documented order, kinds are processed containers-first, and members are
//! restricted to children of already matched containers. Candidates are found through hash buckets
//! and a shingle-bucket index, so the work is proportional to the number of plausible pairs rather
//! than to the square of the pool size.

use std::collections::BTreeMap;

use review_core::ids::SymbolId;
use review_core::matcher::{rule_match, CandidateEdge, MatcherConfig, SymbolRef};
use review_core::symbol::{Hash128, SymbolKind};

use super::RenameHints;

/// The container predicate lives in `review-core`, next to the rule that gives containers their own
/// token rule, so the two cannot drift apart.
pub use review_core::matcher::is_container;

/// The kinds are matched in the documented order: the containers that decide where their members may
/// pair, then the top-level leaves, then the members themselves.
pub const KIND_ORDER: [SymbolKind; 17] = [
    SymbolKind::Module,
    SymbolKind::Class,
    SymbolKind::Interface,
    SymbolKind::Enum,
    SymbolKind::Namespace,
    SymbolKind::Function,
    SymbolKind::Constant,
    SymbolKind::Variable,
    SymbolKind::TypeAlias,
    SymbolKind::Method,
    SymbolKind::Getter,
    SymbolKind::Setter,
    SymbolKind::Property,
    SymbolKind::Field,
    SymbolKind::Constructor,
    SymbolKind::EnumMember,
    SymbolKind::Parameter,
];

/// How many leading shingle hashes form a bucket key (a min-hash sketch).
pub const SHINGLE_BUCKETS: usize = 4;

/// Generates every candidate pair between the removed and added pools.
///
/// `degraded` restricts the run to rule 1, which is what the driver does for pools larger than
/// [`MatcherConfig::degrade_threshold`].
pub fn candidate_edges(
    removed: &[SymbolRef],
    added: &[SymbolRef],
    cfg: &MatcherConfig,
    hints: &RenameHints,
    degraded: bool,
) -> Vec<CandidateEdge> {
    let by_body: BTreeMap<Hash128, Vec<usize>> = index_by(removed, |symbol| symbol.body_hash);
    let by_signature: BTreeMap<Hash128, Vec<usize>> =
        index_by(removed, |symbol| symbol.signature_hash);
    let by_shingle: BTreeMap<u32, Vec<usize>> = index_by_shingles(removed);

    let mut edges: Vec<CandidateEdge> = Vec::new();
    // The decided container pairs, keyed by the added parent, so a member can tell whether it sits
    // inside a matched container.
    let mut matched_parents: BTreeMap<SymbolId, SymbolId> = BTreeMap::new();
    for kind in KIND_ORDER {
        let removed_of_kind: Vec<usize> = (0..removed.len())
            .filter(|index| removed[*index].kind == kind)
            .collect();
        let added_of_kind: Vec<usize> = (0..added.len())
            .filter(|index| added[*index].kind == kind)
            .collect();
        if removed_of_kind.is_empty() || added_of_kind.is_empty() {
            continue;
        }
        for to in &added_of_kind {
            let added_symbol = &added[*to];
            let parent_match = parent_pair(added, *to, &matched_parents);
            for from in plausible(
                removed,
                added,
                *to,
                &by_body,
                &by_signature,
                &by_shingle,
                cfg,
            ) {
                let removed_symbol = &removed[from];
                let inside_matched_parent = match &parent_match {
                    Some(pair) => {
                        removed_symbol.parent_id.as_ref() == Some(&pair.0)
                            && added_symbol.parent_id.as_ref() == Some(&pair.1)
                    }
                    None => false,
                };
                if removed_symbol.kind != added_symbol.kind {
                    continue;
                }
                if is_member(kind) && !inside_matched_parent {
                    // Members of an unmatched container may only pair by an identical body that is
                    // large enough to be meaningful; this is what stops copy-pasted small methods
                    // from cross-matching between unrelated classes.
                    let exact = removed_symbol.body_hash == added_symbol.body_hash
                        && removed_symbol
                            .body_token_count
                            .min(added_symbol.body_token_count)
                            >= cfg.min_tokens_exact;
                    if !exact {
                        continue;
                    }
                }
                let Some(mut edge) = rule_match(removed_symbol, added_symbol, cfg) else {
                    continue;
                };
                edge.from = from;
                edge.to = *to;
                edge.same_parent_match = inside_matched_parent;
                edge.from_rename_hint = hints.is_renamed(
                    removed_symbol.module_path.as_str(),
                    added_symbol.module_path.as_str(),
                );
                edges.push(edge);
            }
        }
        if is_container(kind) {
            record_matched_containers(removed, added, kind, &edges, &mut matched_parents);
        }
    }
    if degraded {
        edges.retain(|edge| edge.rule == review_core::matcher::MatchRule::ExactBody);
    }
    edges
}

/// Remembers the container pairs of one kind that nothing else distinguishes, keyed by the added
/// parent. A container with several equally plausible predecessors has not been decided, so it must
/// not unlock member pairing either.
fn record_matched_containers(
    removed: &[SymbolRef],
    added: &[SymbolRef],
    kind: SymbolKind,
    edges: &[CandidateEdge],
    matched_parents: &mut BTreeMap<SymbolId, SymbolId>,
) {
    let candidates: Vec<&CandidateEdge> = edges
        .iter()
        .filter(|edge| removed[edge.from].kind == kind && added[edge.to].kind == kind)
        .collect();
    let mut per_added: BTreeMap<usize, usize> = BTreeMap::new();
    for edge in &candidates {
        *per_added.entry(edge.to).or_default() += 1;
    }
    for edge in candidates {
        if per_added.get(&edge.to) == Some(&1) {
            matched_parents.insert(added[edge.to].id.clone(), removed[edge.from].id.clone());
        }
    }
}

/// Whether a kind is a member of a container.
pub const fn is_member(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Method
            | SymbolKind::Getter
            | SymbolKind::Setter
            | SymbolKind::Property
            | SymbolKind::Field
            | SymbolKind::Constructor
            | SymbolKind::EnumMember
    )
}

type SymbolIdPair = (SymbolId, SymbolId);

/// The matched parent pair of an added symbol, if its parent was matched in this run.
fn parent_pair(
    added: &[SymbolRef],
    to: usize,
    matched_parents: &BTreeMap<SymbolId, SymbolId>,
) -> Option<SymbolIdPair> {
    let parent = added[to].parent_id.as_ref()?;
    matched_parents
        .get(parent)
        .cloned()
        .map(|removed_parent| (removed_parent, parent.clone()))
}

/// Removed indices that could pair with one added symbol, found through hash buckets.
fn plausible(
    removed: &[SymbolRef],
    added: &[SymbolRef],
    to: usize,
    by_body: &BTreeMap<Hash128, Vec<usize>>,
    by_signature: &BTreeMap<Hash128, Vec<usize>>,
    by_shingle: &BTreeMap<u32, Vec<usize>>,
    cfg: &MatcherConfig,
) -> Vec<usize> {
    let symbol = &added[to];
    let mut candidates: Vec<usize> = Vec::new();
    if let Some(bucket) = by_body.get(&symbol.body_hash) {
        candidates.extend(bucket.iter().copied());
    }
    if let Some(bucket) = by_signature.get(&symbol.signature_hash) {
        candidates.extend(bucket.iter().copied());
    }
    for shingle in leading_shingles(&symbol.shingles) {
        if let Some(bucket) = by_shingle.get(&shingle) {
            candidates.extend(bucket.iter().copied());
        }
    }
    candidates.retain(|index| removed[*index].kind == symbol.kind);
    candidates.sort_unstable();
    candidates.dedup();
    candidates.truncate(cfg.max_candidates);
    candidates
}

fn index_by(
    refs: &[SymbolRef],
    key: impl Fn(&SymbolRef) -> Hash128,
) -> BTreeMap<Hash128, Vec<usize>> {
    let mut out: BTreeMap<Hash128, Vec<usize>> = BTreeMap::new();
    for (index, symbol) in refs.iter().enumerate() {
        out.entry(key(symbol)).or_default().push(index);
    }
    out
}

fn index_by_shingles(refs: &[SymbolRef]) -> BTreeMap<u32, Vec<usize>> {
    let mut out: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, symbol) in refs.iter().enumerate() {
        for shingle in leading_shingles(&symbol.shingles) {
            out.entry(shingle).or_default().push(index);
        }
    }
    out
}

/// The first [`SHINGLE_BUCKETS`] hashes of a sketch, which approximate its similarity locality.
fn leading_shingles(shingles: &review_core::symbol::ShingleSet) -> Vec<u32> {
    shingles.0.iter().take(SHINGLE_BUCKETS).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_core::language::Language;
    use review_core::location::RepoPath;
    use review_core::symbol::{ShingleSet, SymbolKind as Kind};
    use review_core::symbol_id::{module_path_for, SymbolIdParts};

    pub(crate) fn symbol(
        module: &str,
        name: &str,
        kind: Kind,
        body: u8,
        tokens: u32,
        shingles: Vec<u32>,
    ) -> SymbolRef {
        let path = RepoPath::new(format!("{module}.ts")).unwrap();
        let module_path = module_path_for(&path, Language::Typescript);
        let id = SymbolIdParts::new("ts", module_path.clone(), vec![name.to_owned()], kind)
            .format()
            .unwrap();
        SymbolRef {
            key: review_core::ids::SymbolKey::of(&id),
            id,
            kind,
            name: name.to_owned(),
            qualified_name: vec![name.to_owned()],
            module_path,
            parent_id: None,
            signature_hash: Hash128::of("sig", &[0]),
            body_hash: Hash128::of("body", &[body]),
            body_token_count: tokens,
            shingles: ShingleSet::of(shingles),
        }
    }

    #[test]
    fn containers_and_members_are_classified() {
        assert!(is_container(Kind::Class));
        assert!(is_container(Kind::Module));
        assert!(is_member(Kind::Method));
        assert!(is_member(Kind::EnumMember));
        assert!(!is_member(Kind::Function));
    }

    #[test]
    fn exact_bodies_produce_candidates_and_other_bodies_do_not() {
        let cfg = MatcherConfig::default();
        let hints = RenameHints::default();
        let removed = vec![
            symbol("src/a", "one", Kind::Function, 1, 30, vec![1, 2, 3]),
            symbol("src/a", "two", Kind::Function, 2, 30, vec![4, 5, 6]),
        ];
        let added = vec![symbol(
            "src/b",
            "three",
            Kind::Function,
            1,
            30,
            vec![1, 2, 3],
        )];
        let edges = candidate_edges(&removed, &added, &cfg, &hints, false);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, 0);
        assert_eq!(edges[0].to, 0);
        assert_eq!(edges[0].rule, review_core::matcher::MatchRule::ExactBody);
    }

    #[test]
    fn degraded_mode_keeps_only_rule_one() {
        let cfg = MatcherConfig::default();
        let hints = RenameHints::default();
        let removed = vec![symbol(
            "src/a",
            "helper",
            Kind::Function,
            9,
            30,
            vec![1, 2, 3],
        )];
        let added = vec![symbol(
            "src/b",
            "helper",
            Kind::Function,
            7,
            30,
            vec![1, 2, 3],
        )];
        let full = candidate_edges(&removed, &added, &cfg, &hints, false);
        assert!(
            full.iter()
                .any(|edge| edge.rule == review_core::matcher::MatchRule::SignatureAndName),
            "without degradation rule 2 fires"
        );
        let degraded = candidate_edges(&removed, &added, &cfg, &hints, true);
        assert!(degraded.is_empty(), "rule 2 cannot fire in degraded mode");
    }

    #[test]
    fn members_of_unmatched_containers_need_an_exact_large_body() {
        let cfg = MatcherConfig::default();
        let hints = RenameHints::default();
        let removed = [symbol("src/a", "A", Kind::Class, 1, 40, vec![1, 2, 3])];
        let mut method = symbol("src/a", "m", Kind::Method, 5, 30, vec![7, 8, 9]);
        method.parent_id = Some(removed[0].id.clone());
        let mut added = vec![symbol("src/b", "B", Kind::Class, 2, 40, vec![4, 5, 6])];
        let mut other = symbol("src/b", "m", Kind::Method, 6, 4, vec![7, 8, 9]);
        other.parent_id = Some(added[0].id.clone());
        added.push(other);
        let pool = vec![removed[0].clone(), method];
        let edges = candidate_edges(&pool, &added, &cfg, &hints, false);
        assert!(
            edges
                .iter()
                .all(|edge| removed_symbol_kind(edge, &pool) != Kind::Method),
            "a tiny member of an unmatched container never pairs: {edges:?}"
        );
    }

    fn removed_symbol_kind(edge: &CandidateEdge, removed: &[SymbolRef]) -> Kind {
        removed[edge.from].kind
    }
}
