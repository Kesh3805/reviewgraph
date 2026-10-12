#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use context_engine::model::*;
use review_core::ids::SymbolKey;
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

fn item(title: &str, score: f32, pinned: bool) -> ContextItem {
    let key = SymbolKey::from_bytes([title.len() as u8; 16]);
    ContextItem {
        id: ItemId::of(ItemKind::RelatedSymbol, Some(&key), DiffSide::Head, title),
        kind: ItemKind::RelatedSymbol,
        node: Some(key),
        side: DiffSide::Head,
        title: title.to_string(),
        relation: None,
        distance: Some(1),
        path: vec![],
        sources: BTreeSet::from([CandidateSource::Structural]),
        signals: SignalVector::default(),
        score,
        content: ItemContent::Full { text: "x".into() },
        locations: vec![],
        tokens: 10,
        pinned,
    }
}

fn omitted(n: usize) -> Vec<OmittedItem> {
    (0..n)
        .map(|i| OmittedItem {
            node: None,
            title: format!("o{i:04}"),
            kind: ItemKind::RelatedSymbol,
            score: 0.1,
            reason: OmitReason::BudgetTokens,
        })
        .collect()
}

fn package(items: Vec<ContextItem>, om: Vec<OmittedItem>) -> ContextPackage {
    ContextPackage::new(
        ReviewerType::Security,
        "c1",
        items,
        om,
        ContextBudget {
            max_tokens: 100,
            max_items: 10,
        },
        vec![],
        ChangeSummary::default(),
        ContextProvenance::default(),
    )
}

#[test]
fn item_id_stable() {
    let k = SymbolKey::from_bytes([7; 16]);
    let a = ItemId::of(ItemKind::Test, Some(&k), DiffSide::Base, "t");
    assert_eq!(a, ItemId::of(ItemKind::Test, Some(&k), DiffSide::Base, "t"));
    assert_ne!(a, ItemId::of(ItemKind::Test, Some(&k), DiffSide::Head, "t"));
    assert_eq!(a.to_hex().len(), 32);
}

#[test]
fn sorted_order_deterministic() {
    let p = package(
        vec![
            item("a", 0.9, false),
            item("bb", 0.1, true),
            item("ccc", 0.5, false),
        ],
        vec![],
    );
    let titles: Vec<_> = p.items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["bb", "a", "ccc"]);
}

#[test]
fn omitted_capped_with_overflow_count() {
    let p = package(vec![], omitted(250));
    assert_eq!(p.omitted.len(), MAX_OMITTED_LISTED);
    assert_eq!(p.omitted_overflow, 50);
}

#[test]
fn model_input_has_all_prd_89_sections() {
    let v = package(vec![item("a", 0.5, false)], vec![]).to_model_input();
    for k in [
        "changedSymbols",
        "changeSummary",
        "graphNeighborhood",
        "tests",
        "repositoryRules",
        "riskSignals",
        "deterministicFindings",
    ] {
        assert!(v.get(k).is_some(), "missing {k}");
    }
}

#[test]
fn every_excerpt_has_location() {
    let v = package(vec![item("a", 0.5, false)], vec![]).to_model_input();
    let n = &v["graphNeighborhood"][0];
    assert!(n.get("location").is_some());
}

#[test]
fn schema_roundtrip() {
    let p = package(vec![item("a", 0.5, false)], omitted(2));
    let json = serde_json::to_string(&p).unwrap();
    let back: ContextPackage = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
}
