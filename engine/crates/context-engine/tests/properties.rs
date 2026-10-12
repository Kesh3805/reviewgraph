//! Property tests for budgeting and assembly invariants (CTX-006, CTX-008, CTX-010).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeSet;

use common::*;
use context_engine::assemble::assemble;
use context_engine::budget::{
    fill_remaining, select, BudgetLimits, Candidate as BudgetCandidate, RESERVE_TOKENS,
};
use context_engine::candidates::{Candidate, CandidatePool, RawSignals};
use context_engine::model::{
    CandidateSource, ContextItem, ItemContent, ItemId, ItemKind, OmitReason, SignalVector,
};
use proptest::prelude::*;
use review_core::ids::SymbolKey;
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

const KINDS: [ItemKind; 10] = [
    ItemKind::ChangedSymbol,
    ItemKind::RelatedSymbol,
    ItemKind::Test,
    ItemKind::Config,
    ItemKind::ApiEndpoint,
    ItemKind::RuleDoc,
    ItemKind::Convention,
    ItemKind::SchemaChange,
    ItemKind::DependencyChange,
    ItemKind::ClassSignature,
];

#[derive(Debug, Clone)]
struct Spec {
    kind: usize,
    score: f32,
    tokens: u32,
    pinned: bool,
    cheaper: Option<u32>,
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        0..KINDS.len(),
        0.0f32..1.0,
        1u32..4_000,
        any::<bool>(),
        proptest::option::of(1u32..2_000),
    )
        .prop_map(|(kind, score, tokens, pinned, cheaper)| Spec {
            kind,
            score,
            tokens,
            pinned,
            cheaper,
        })
}

fn limits() -> impl Strategy<Value = BudgetLimits> {
    (
        1u16..10,
        0u16..30,
        0u16..10,
        0u16..8,
        0u16..8,
        100u32..60_000,
    )
        .prop_map(
            |(changed, related, tests, configs, rules, max_tokens)| BudgetLimits {
                max_changed: changed,
                max_related: related,
                max_tests: tests,
                max_configs: configs,
                max_rule_docs: rules,
                max_tokens,
                reserve_tokens: RESERVE_TOKENS,
            },
        )
}

fn budget_candidates(specs: &[Spec]) -> Vec<BudgetCandidate> {
    specs
        .iter()
        .enumerate()
        .map(|(n, s)| {
            let kind = KINDS[s.kind];
            let key = SymbolKey::from_bytes([n as u8 + 1; 16]);
            let title = format!("sym{n}");
            BudgetCandidate {
                item: ContextItem {
                    id: ItemId::of(kind, Some(&key), DiffSide::Head, &title),
                    kind,
                    node: Some(key),
                    side: DiffSide::Head,
                    title,
                    relation: None,
                    distance: Some(1),
                    path: vec![],
                    sources: BTreeSet::from([CandidateSource::Structural]),
                    signals: SignalVector::default(),
                    score: s.score,
                    content: ItemContent::Full { text: "x".into() },
                    locations: vec![],
                    tokens: s.tokens,
                    pinned: s.pinned,
                },
                cheaper: s.cheaper.map(|t| (ItemContent::Empty, t)),
            }
        })
        .collect()
}

fn category(kind: ItemKind) -> usize {
    match kind {
        ItemKind::ChangedSymbol | ItemKind::ClassSignature => 0,
        ItemKind::RelatedSymbol
        | ItemKind::ApiEndpoint
        | ItemKind::SchemaChange
        | ItemKind::DependencyChange => 1,
        ItemKind::Test => 2,
        ItemKind::Config => 3,
        ItemKind::RuleDoc | ItemKind::Convention => 4,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn prop_budget_never_exceeded(specs in proptest::collection::vec(spec(), 0..40), l in limits()) {
        let sel = select(budget_candidates(&specs), &l);
        let used: u32 = sel.selected.iter().map(|i| i.tokens).sum();
        prop_assert!(used <= l.max_tokens);
        prop_assert_eq!(used, sel.tokens_used);
    }

    #[test]
    fn prop_counts_within_caps(specs in proptest::collection::vec(spec(), 0..40), l in limits()) {
        let sel = select(budget_candidates(&specs), &l);
        let caps = [l.max_changed, l.max_related, l.max_tests, l.max_configs, l.max_rule_docs];
        let mut counts = [0usize; 5];
        for item in &sel.selected {
            counts[category(item.kind)] += 1;
        }
        for (count, cap) in counts.iter().zip(caps) {
            prop_assert!(*count <= usize::from(cap));
        }
    }

    #[test]
    fn prop_selected_xor_omitted(specs in proptest::collection::vec(spec(), 0..40), l in limits()) {
        let input = budget_candidates(&specs);
        let all: BTreeSet<ItemId> = input.iter().map(|c| c.item.id).collect();
        let sel = select(input, &l);
        let selected: Vec<ItemId> = sel.selected.iter().map(|i| i.id).collect();
        prop_assert_eq!(selected.len() + sel.omitted.len(), all.len());
        let uniq: BTreeSet<ItemId> = selected.iter().copied().collect();
        prop_assert_eq!(uniq.len(), selected.len());
        prop_assert!(uniq.is_subset(&all));
    }

    #[test]
    fn prop_pinned_present(specs in proptest::collection::vec(spec(), 0..40), l in limits()) {
        let input = budget_candidates(&specs);
        let pinned: Vec<(ItemId, String)> = input
            .iter()
            .filter(|c| c.item.pinned)
            .map(|c| (c.item.id, c.item.title.clone()))
            .collect();
        let sel = select(input, &l);
        let present: BTreeSet<ItemId> = sel.selected.iter().map(|i| i.id).collect();
        for (id, title) in pinned {
            if !present.contains(&id) {
                let reason = sel
                    .omitted
                    .iter()
                    .find(|o| o.title == title)
                    .map(|o| o.reason.clone());
                prop_assert!(
                    matches!(
                        reason,
                        Some(OmitReason::BudgetTokens | OmitReason::BudgetCount { .. })
                    ),
                    "pinned {} dropped without a budget reason: {:?}", title, reason
                );
            }
        }
    }

    #[test]
    fn prop_semantic_only_adds(
        first in proptest::collection::vec(spec(), 0..20),
        extra in proptest::collection::vec(spec(), 0..20),
        l in limits(),
    ) {
        let sel = select(budget_candidates(&first), &l);
        let before: BTreeSet<ItemId> = sel.selected.iter().map(|i| i.id).collect();
        let mut extra_cands = budget_candidates(&extra);
        for (n, c) in extra_cands.iter_mut().enumerate() {
            let key = SymbolKey::from_bytes([n as u8 + 100; 16]);
            c.item.kind = ItemKind::RelatedSymbol;
            c.item.pinned = false;
            c.item.node = Some(key);
            c.item.title = format!("semantic{n}");
            c.item.id = ItemId::of(ItemKind::RelatedSymbol, Some(&key), DiffSide::Head, &c.item.title);
        }
        let filled = fill_remaining(sel, extra_cands, &l);
        let after: BTreeSet<ItemId> = filled.selected.iter().map(|i| i.id).collect();
        prop_assert!(before.is_subset(&after));
        prop_assert!(filled.tokens_used <= l.max_tokens);
    }
}

fn pool_from(order: &[usize], defs: &[(u8, f32, bool)]) -> CandidatePool {
    let mut pool = CandidatePool::new();
    for &i in order {
        let (distance, structural, weak) = defs[i];
        let id = format!("ts:src/p/p{i}.ts#P{i}.run");
        let mut c = Candidate::node(
            key(&id),
            format!("P{i}.run"),
            ItemKind::RelatedSymbol,
            DiffSide::Head,
            CandidateSource::Structural,
        );
        c.distance = Some(distance % 5);
        c.weak = weak;
        c.raw = RawSignals {
            structural,
            ..RawSignals::default()
        };
        pool.insert(c);
    }
    pool
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1_000))]

    #[test]
    fn prop_hash_permutation_invariant(
        defs in proptest::collection::vec((0u8..6, 0.0f32..1.0, any::<bool>()), 1..25),
        seed in any::<u64>(),
    ) {
        let n = defs.len();
        let forward: Vec<usize> = (0..n).collect();
        let mut shuffled = forward.clone();
        let mut state = seed | 1;
        for i in (1..n).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let j = usize::try_from(state % (i as u64 + 1)).unwrap();
            shuffled.swap(i, j);
        }
        let graph = graph_of(vec![]);
        let input = input(ReviewerType::Correctness, Some(&graph));
        let blobs = blobs();
        let opts = options(&blobs, ReviewerType::Correctness);
        let a = assemble(&input, pool_from(&forward, &defs), &opts);
        let b = assemble(&input, pool_from(&shuffled, &defs), &opts);
        prop_assert_eq!(a.package_hash, b.package_hash);
        prop_assert!(a.token_estimate <= a.budget.max_tokens);
    }
}
