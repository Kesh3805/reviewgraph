#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

use common::*;
use context_engine::assemble::{assemble, generate, AssembleOptions};
use context_engine::budget::{resolve, semantic_item_cap};
use context_engine::candidates::semantic::{
    SemanticHit, SemanticRequest, SemanticSource, SemanticUnavailable,
};
use context_engine::model::{ContextPackage, ItemId, SemanticStatus};
use review_core::reviewer_type::ReviewerType;

#[derive(Default)]
struct Fake {
    hits: Vec<SemanticHit>,
    error: Option<SemanticUnavailable>,
    calls: Cell<u32>,
    last: RefCell<Option<SemanticRequest>>,
}

impl SemanticSource for Fake {
    fn search(&self, request: &SemanticRequest) -> Result<Vec<SemanticHit>, SemanticUnavailable> {
        self.calls.set(self.calls.get() + 1);
        *self.last.borrow_mut() = Some(request.clone());
        match &self.error {
            Some(e) => Err(e.clone()),
            None => Ok(self.hits.clone()),
        }
    }
}

fn hit(n: u8, cosine: f32) -> SemanticHit {
    SemanticHit {
        node: key(&format!("ts:src/sibling/s{n}.ts#Sibling{n}.authorize")),
        title: format!("Sibling{n}.authorize"),
        cosine,
        span: None,
    }
}

fn run(
    reviewer: ReviewerType,
    fake: Option<&Fake>,
    tweak: impl FnOnce(&mut AssembleOptions<'_>),
) -> ContextPackage {
    let graph = graph_of(auth_bypass_elements());
    let locator = locator();
    let mut input = input(reviewer, Some(&graph));
    input.locator = Some(&locator);
    let (pool, flags) = generate(&input, None);
    let blobs = blobs();
    let mut opts = options(&blobs, reviewer);
    opts.flags = flags;
    opts.semantic = fake.map(|f| f as &dyn SemanticSource);
    tweak(&mut opts);
    assemble(&input, pool, &opts)
}

fn ids(pkg: &ContextPackage) -> BTreeSet<ItemId> {
    pkg.items.iter().map(|i| i.id).collect()
}

#[test]
fn semantic_never_removes_structural_items() {
    let fake = Fake {
        hits: vec![hit(1, 0.9), hit(2, 0.8)],
        ..Fake::default()
    };
    let without = run(ReviewerType::Correctness, None, |_| {});
    let with = run(ReviewerType::Correctness, Some(&fake), |_| {});
    assert!(ids(&without).is_subset(&ids(&with)));
    assert!(with.items.len() > without.items.len());
    assert!(matches!(
        with.provenance.semantic,
        SemanticStatus::Added { count: 2 }
    ));
}

#[test]
fn not_needed_when_budget_full() {
    let fake = Fake {
        hits: vec![hit(1, 0.9)],
        ..Fake::default()
    };
    let pkg = run(ReviewerType::Correctness, Some(&fake), |o| {
        // Leaves less than the 1,500 tokens a fill needs.
        o.limits.max_tokens = 1_000;
    });
    assert_eq!(fake.calls.get(), 0, "no query is made");
    assert!(matches!(
        pkg.provenance.semantic,
        SemanticStatus::NotNeeded { .. }
    ));
}

#[test]
fn fill_limited_to_25pct_related() {
    let fake = Fake {
        hits: (1..=12).map(|n| hit(n, 0.9)).collect(),
        ..Fake::default()
    };
    let limits = resolve(ReviewerType::Correctness, 1.0, u32::MAX, u32::MAX).unwrap();
    let cap = semantic_item_cap(&limits);
    assert_eq!(cap, 5);
    let pkg = run(ReviewerType::Correctness, Some(&fake), |_| {});
    let sibling = pkg
        .items
        .iter()
        .filter(|i| i.title.starts_with("Sibling"))
        .count();
    assert_eq!(sibling, cap);
}

#[test]
fn timeout_degrades_with_provenance() {
    let fake = Fake {
        error: Some(SemanticUnavailable::Timeout),
        ..Fake::default()
    };
    let without = run(ReviewerType::Correctness, None, |_| {});
    let with = run(ReviewerType::Correctness, Some(&fake), |_| {});
    assert_eq!(ids(&without), ids(&with));
    match &with.provenance.semantic {
        SemanticStatus::Unavailable { reason } => assert!(reason.contains("timed out")),
        other => panic!("expected Unavailable, got {other:?}"),
    }
}

#[test]
fn excludes_seeds_and_selected() {
    let fake = Fake {
        hits: vec![hit(1, 0.9)],
        ..Fake::default()
    };
    let pkg = run(ReviewerType::Correctness, Some(&fake), |_| {});
    let request = fake.last.borrow().clone().unwrap();
    assert!(request.exclude.contains(&key(AUTHORIZE)));
    assert!(request.exclude.contains(&key(UPDATE_USER)));
    assert_eq!(
        pkg.items
            .iter()
            .filter(|i| i.node == Some(key(UPDATE_USER)))
            .count(),
        1
    );
}

#[test]
fn hits_below_the_cosine_floor_are_dropped() {
    let fake = Fake {
        hits: vec![hit(1, 0.4)],
        ..Fake::default()
    };
    let pkg = run(ReviewerType::Correctness, Some(&fake), |_| {});
    assert!(pkg.items.iter().all(|i| !i.title.starts_with("Sibling")));
}

#[test]
fn disabled_semantic_is_recorded() {
    let pkg = run(ReviewerType::Correctness, None, |o| {
        o.semantic_disabled = Some("privacy mode forbids remote embeddings".to_owned());
    });
    assert!(matches!(
        pkg.provenance.semantic,
        SemanticStatus::Unavailable { .. }
    ));
}
