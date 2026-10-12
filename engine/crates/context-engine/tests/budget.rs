#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use context_engine::budget::*;
use context_engine::model::*;
use review_core::ids::SymbolKey;
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

fn cand(n: u32, kind: ItemKind, score: f32, tokens: u32, pinned: bool) -> Candidate {
    let key = SymbolKey::from_bytes([n as u8; 16]);
    let title = format!("sym{n}");
    Candidate {
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
            score,
            content: ItemContent::Full { text: "x".into() },
            locations: vec![],
            tokens,
            pinned,
        },
        cheaper: None,
    }
}

fn limits(max_tokens: u32) -> BudgetLimits {
    BudgetLimits {
        max_changed: 8,
        max_related: 20,
        max_tests: 8,
        max_configs: 4,
        max_rule_docs: 4,
        max_tokens,
        reserve_tokens: RESERVE_TOKENS,
    }
}

#[test]
fn correctness_defaults_8_20_8_4() {
    let l = resolve(ReviewerType::Correctness, 1.0, u32::MAX, u32::MAX).unwrap();
    assert_eq!(
        (l.max_changed, l.max_related, l.max_tests, l.max_configs),
        (8, 20, 8, 4)
    );
}

#[test]
fn risk_multiplier_scales_tokens_and_related() {
    let base = resolve(ReviewerType::Correctness, 1.0, u32::MAX, u32::MAX).unwrap();
    let big = resolve(ReviewerType::Correctness, 1.5, u32::MAX, u32::MAX).unwrap();
    assert!(big.max_tokens > base.max_tokens);
    assert_eq!(big.max_related, 30);
    assert_eq!(big.max_changed, base.max_changed);
}

#[test]
fn config_max_context_tokens_clamps() {
    let l = resolve(ReviewerType::Correctness, 1.0, 10_000, u32::MAX).unwrap();
    assert_eq!(l.max_tokens, 10_000 - RESERVE_TOKENS);
}

#[test]
fn budget_exhausted_error() {
    assert_eq!(
        resolve(ReviewerType::Security, 1.0, u32::MAX, 0),
        Err(BudgetError::BudgetExhausted)
    );
    assert_eq!(
        resolve(ReviewerType::Security, 1.0, u32::MAX, RESERVE_TOKENS),
        Err(BudgetError::BudgetExhausted)
    );
}

#[test]
fn estimate_tokens_formula() {
    assert_eq!(estimate_tokens(""), 8);
    assert_eq!(estimate_tokens("abcd"), 9);
    assert_eq!(estimate_tokens("abcde"), 10);
}

#[test]
fn pinned_first() {
    let c = vec![
        cand(1, ItemKind::RelatedSymbol, 0.99, 60, false),
        cand(2, ItemKind::ChangedSymbol, 0.1, 60, true),
    ];
    let s = select(c, &limits(100));
    assert_eq!(s.selected.len(), 1);
    assert!(s.selected[0].pinned);
    assert_eq!(s.omitted[0].reason, OmitReason::BudgetTokens);
}

#[test]
fn pinned_overflow_drops_lowest_risk_with_reason() {
    let c = vec![
        cand(1, ItemKind::ChangedSymbol, 0.9, 60, true),
        cand(2, ItemKind::ChangedSymbol, 0.2, 60, true),
    ];
    let s = select(c, &limits(100));
    assert_eq!(s.selected.len(), 1);
    assert_eq!(s.selected[0].title, "sym1");
    assert_eq!(s.omitted[0].title, "sym2");
    assert_eq!(s.omitted[0].reason, OmitReason::BudgetTokens);
}

#[test]
fn greedy_by_score_per_token() {
    // a: 0.6 / 80 = 0.0075, b: 0.4 / 20 = 0.02, c: 0.3 / 20 = 0.015
    let c = vec![
        cand(1, ItemKind::RelatedSymbol, 0.6, 80, false),
        cand(2, ItemKind::RelatedSymbol, 0.4, 20, false),
        cand(3, ItemKind::RelatedSymbol, 0.3, 20, false),
    ];
    let s = select(c, &limits(100));
    let titles: BTreeSet<_> = s.selected.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, BTreeSet::from(["sym2", "sym3"]));
    assert_eq!(s.omitted.len(), 1);
}

#[test]
fn cheaper_form_used_when_default_does_not_fit() {
    let mut c = cand(1, ItemKind::RelatedSymbol, 0.7, 200, false);
    c.cheaper = Some((ItemContent::Signature { text: "sig".into() }, 30));
    let s = select(vec![c], &limits(100));
    assert_eq!(s.selected.len(), 1);
    assert_eq!(s.selected[0].tokens, 30);
}

#[test]
fn cheaper_form_not_used_below_half_score() {
    let mut c = cand(1, ItemKind::RelatedSymbol, 0.4, 200, false);
    c.cheaper = Some((ItemContent::Signature { text: "sig".into() }, 30));
    let s = select(vec![c], &limits(100));
    assert!(s.selected.is_empty());
}

#[test]
fn category_caps_respected() {
    let mut l = limits(10_000);
    l.max_tests = 2;
    let c = (0..5)
        .map(|n| cand(n, ItemKind::Test, 0.5, 10, false))
        .collect();
    let s = select(c, &l);
    assert_eq!(s.selected.len(), 2);
    assert_eq!(s.omitted.len(), 3);
    assert!(s
        .omitted
        .iter()
        .all(|o| matches!(o.reason, OmitReason::BudgetCount { .. })));
}

#[test]
fn every_unselected_candidate_omitted_with_reason() {
    let c: Vec<_> = (0..30)
        .map(|n| cand(n, ItemKind::RelatedSymbol, 0.5, 40, false))
        .collect();
    let total = c.len();
    let s = select(c, &limits(300));
    assert_eq!(s.selected.len() + s.omitted.len(), total);
}

/// Deterministic pseudo-random sweep (10,000 cases): budgets are never exceeded and every
/// candidate is selected xor omitted.
#[test]
fn tokens_never_exceed_max_sweep() {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move |m: u64| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % m
    };
    let kinds = [
        ItemKind::ChangedSymbol,
        ItemKind::RelatedSymbol,
        ItemKind::Test,
        ItemKind::Config,
        ItemKind::RuleDoc,
        ItemKind::ClassSignature,
    ];
    for _ in 0..10_000 {
        let n = next(40) as u32;
        let cands: Vec<_> = (0..n)
            .map(|i| {
                let kind = kinds[next(6) as usize];
                let mut c = cand(
                    i,
                    kind,
                    next(100) as f32 / 100.0,
                    next(500) as u32 + 1,
                    next(4) == 0,
                );
                if next(2) == 0 {
                    c.cheaper = Some((ItemContent::Empty, next(50) as u32 + 1));
                }
                c
            })
            .collect();
        let max_tokens = next(1500) as u32 + 1;
        let s = select(cands, &limits(max_tokens));
        let sum: u32 = s.selected.iter().map(|i| i.tokens).sum();
        assert!(sum <= max_tokens);
        assert_eq!(sum, s.tokens_used);
        assert_eq!(s.selected.len() + s.omitted.len(), n as usize);
    }
}
