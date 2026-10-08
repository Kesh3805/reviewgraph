//! One-to-one assignment of candidate pairs (SID-005).
//!
//! Candidates are sorted by the documented tie-breakers and taken greedily while both endpoints are
//! free. That is stable, needs no Hungarian algorithm at this scale, and produces the same pairing
//! for the same input set regardless of the order the candidates were generated in.

use std::cmp::Reverse;

use review_core::matcher::{CandidateEdge, MatcherConfig, SymbolRef};

/// Returns the edges that were accepted, in assignment order.
pub fn assign(
    _removed: &[SymbolRef],
    _added: &[SymbolRef],
    edges: Vec<CandidateEdge>,
    _cfg: &MatcherConfig,
) -> Vec<CandidateEdge> {
    let mut ordered = edges;
    ordered.sort_by_key(|edge| Reverse(edge.ordering()));
    let mut taken_removed: Vec<bool> = vec![false; _removed.len()];
    let mut taken_added: Vec<bool> = vec![false; _added.len()];
    let mut accepted = Vec::with_capacity(ordered.len().min(_added.len()));
    for edge in ordered {
        if taken_removed[edge.from] || taken_added[edge.to] {
            continue;
        }
        taken_removed[edge.from] = true;
        taken_added[edge.to] = true;
        accepted.push(edge);
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_core::matcher::MatchRule;
    use review_core::symbol::SymbolKind;

    fn edge(from: usize, to: usize, similarity: f32, same_name: bool) -> CandidateEdge {
        CandidateEdge {
            from,
            to,
            rule: MatchRule::TokenSimilarity,
            similarity,
            same_parent_match: false,
            same_name,
            directory_distance: 0,
            from_rename_hint: false,
        }
    }

    fn symbol(name: &str) -> SymbolRef {
        use review_core::ids::SymbolKey;
        use review_core::language::Language;
        use review_core::location::RepoPath;
        use review_core::symbol::{Hash128, ModulePath, ShingleSet};
        use review_core::symbol_id::{module_path_for, SymbolIdParts};
        let path = RepoPath::new(format!("src/{name}.ts")).unwrap();
        let module_path: ModulePath = module_path_for(&path, Language::Typescript);
        let id = SymbolIdParts::new(
            "ts",
            module_path.clone(),
            vec![name.to_owned()],
            SymbolKind::Function,
        )
        .format()
        .unwrap();
        SymbolRef {
            key: SymbolKey::of(&id),
            id,
            kind: SymbolKind::Function,
            name: name.to_owned(),
            qualified_name: vec![name.to_owned()],
            module_path,
            parent_id: None,
            signature_hash: Hash128::ZERO,
            body_hash: Hash128::ZERO,
            body_token_count: 0,
            shingles: ShingleSet::default(),
        }
    }

    #[test]
    fn the_best_free_pair_wins_and_pairs_stay_one_to_one() {
        let removed = vec![symbol("a"), symbol("b")];
        let added = vec![symbol("c")];
        let edges = vec![edge(1, 0, 0.8, false), edge(0, 0, 0.95, true)];
        let accepted = assign(&removed, &added, edges, &MatcherConfig::default());
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].from, 0);
    }

    #[test]
    fn input_order_does_not_change_the_result() {
        let removed = vec![symbol("a"), symbol("b"), symbol("c")];
        let added = vec![symbol("d"), symbol("e")];
        let forward = vec![
            edge(0, 0, 0.9, true),
            edge(1, 1, 0.85, true),
            edge(2, 0, 0.9, true),
        ];
        let backward: Vec<CandidateEdge> = forward.iter().rev().cloned().collect();
        let cfg = MatcherConfig::default();
        let first = assign(&removed, &added, forward, &cfg);
        let second = assign(&removed, &added, backward, &cfg);
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn assignment_is_independent_of_candidate_order(
            pairs in proptest::collection::vec((0usize..6, 0usize..6, 0.5f32..1.0), 0..24)
        ) {
            let cfg = MatcherConfig::default();
            let removed = vec![
                symbol("a"), symbol("b"), symbol("c"),
                symbol("d"), symbol("e"), symbol("f"),
            ];
            let added = vec![
                symbol("g"), symbol("h"), symbol("i"),
                symbol("j"), symbol("k"), symbol("l"),
            ];
            let edges: Vec<CandidateEdge> = pairs
                .iter()
                .map(|(from, to, similarity)| edge(*from, *to, *similarity, false))
                .collect();
            let mut sorted = edges.clone();
            sorted.sort_by_key(|edge| (edge.from, edge.to));
            let first = assign(&removed, &added, edges, &cfg);
            let second = assign(&removed, &added, sorted, &cfg);
            prop_assert_eq!(first.clone(), second);
            prop_assert!(first.len() <= added.len());
        }
    }
}
