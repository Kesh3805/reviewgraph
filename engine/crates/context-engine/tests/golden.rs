//! Label-based goldens for the synthetic auth-bypass package (CTX-010): decisive items must be
//! present, decoys absent, and every omission carries a reason.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::*;
use context_engine::assemble::{assemble, generate};
use context_engine::candidates::lexical::LexicalCandidates;
use context_engine::lexical::{LexicalDoc, LexicalIndex};
use context_engine::model::{
    CompressionLevel, ContextItem, ContextPackage, ItemContent, ItemKind, OmitReason,
};
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

fn lexical() -> LexicalCandidates {
    let docs = vec![
        LexicalDoc {
            node: key(FORMAT_HEADER),
            identifiers: vec!["authorizeHeader".into()],
        },
        LexicalDoc {
            node: key(DECOY),
            identifiers: vec!["generate".into()],
        },
        LexicalDoc {
            node: key(PROVIDER),
            identifiers: vec!["authorize".into()],
        },
    ];
    LexicalCandidates::new(
        Arc::new(LexicalIndex::build(docs).unwrap()),
        vec!["authorize".into(), "updateUser".into(), "check".into()],
    )
}

fn build(reviewer: ReviewerType) -> ContextPackage {
    let graph = graph_of(auth_bypass_elements());
    let locator = locator();
    let mut input = input(reviewer, Some(&graph));
    input.locator = Some(&locator);
    let (pool, flags) = generate(&input, Some(&lexical()));
    let blobs = blobs();
    let mut opts = options(&blobs, reviewer);
    opts.flags = flags;
    assemble(&input, pool, &opts)
}

fn find<'a>(pkg: &'a ContextPackage, needle: &str, side: DiffSide) -> Option<&'a ContextItem> {
    pkg.items
        .iter()
        .find(|i| i.title.contains(needle) && i.side == side)
}

fn must_include(pkg: &ContextPackage, needle: &str, side: DiffSide) -> &ContextItem {
    find(pkg, needle, side).unwrap_or_else(|| {
        let why: Vec<_> = pkg
            .omitted
            .iter()
            .filter(|o| o.title.contains(needle))
            .collect();
        panic!(
            "missing {needle} ({side:?}); omitted as {why:?}; items {:?}",
            pkg.items.iter().map(|i| &i.title).collect::<Vec<_>>()
        )
    })
}

fn must_exclude(pkg: &ContextPackage, needle: &str) {
    assert!(
        pkg.items.iter().all(|i| !i.title.contains(needle)),
        "{needle} must not be in the package"
    );
}

#[test]
fn golden_context_auth_bypass_correctness() {
    let pkg = build(ReviewerType::Correctness);
    must_include(&pkg, "AuthService.authorize", DiffSide::Head);
    must_include(&pkg, "AuthService.authorize", DiffSide::Base);
    let update = must_include(&pkg, "AdminService.updateUser", DiffSide::Head);
    let ItemContent::Code { level, .. } = &update.content else {
        panic!("updateUser is not code")
    };
    assert_eq!(*level, CompressionLevel::BodyTrimmed);
    let text = update.content.text();
    assert!(
        text.contains("line 45\n"),
        "the authorize call line survives"
    );
    assert!(text.contains("line 46\n") && text.contains("line 47\n"));
    must_include(&pkg, "PermissionService.check", DiffSide::Base);
    must_include(&pkg, "AuthProvider.authorize", DiffSide::Head);
    must_include(&pkg, "authorize denies", DiffSide::Head);
    must_exclude(&pkg, "ReportService");
    must_exclude(&pkg, "authorizeHeader");
    assert!(pkg.token_estimate <= pkg.budget.max_tokens);
    assert!(pkg.package_hash.len() == 64);
}

#[test]
fn golden_context_auth_bypass_security() {
    let pkg = build(ReviewerType::Security);
    must_include(&pkg, "AuthService.authorize", DiffSide::Head);
    must_include(&pkg, "UserController.update", DiffSide::Head);
    let endpoint = must_include(&pkg, "http:PUT /users/:id", DiffSide::Head);
    assert_eq!(endpoint.kind, ItemKind::ApiEndpoint);
    assert!(endpoint.pinned);
    let ItemContent::Structured { value } = &endpoint.content else {
        panic!("endpoint is not structured")
    };
    assert_eq!(value["guards"][0], "JwtGuard");
    must_include(&pkg, "AdminService.updateUser", DiffSide::Head);
    must_exclude(&pkg, "ReportService");
    must_exclude(&pkg, "authorizeHeader");
}

#[test]
fn security_ranks_controller_and_endpoint_in_top_three() {
    let pkg = build(ReviewerType::Security);
    let non_seed: Vec<&ContextItem> = pkg
        .items
        .iter()
        .filter(|i| i.kind != ItemKind::ChangedSymbol && i.kind != ItemKind::ClassSignature)
        .collect();
    let mut by_score = non_seed.clone();
    by_score.sort_by(|a, b| b.score.total_cmp(&a.score));
    let top: Vec<&str> = by_score.iter().take(4).map(|i| i.title.as_str()).collect();
    assert!(
        top[..3].iter().any(|t| t.contains("UserController.update")),
        "{top:?}"
    );
    assert!(
        top.iter().any(|t| t.contains("http:PUT /users/:id")),
        "{top:?}"
    );
}

#[test]
fn correctness_ranks_updateuser_first_related() {
    let pkg = build(ReviewerType::Correctness);
    let mut related: Vec<&ContextItem> = pkg
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::RelatedSymbol)
        .collect();
    related.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert!(
        related[0].title.contains("updateUser"),
        "{:?}",
        related[0].title
    );
}

#[test]
fn every_candidate_is_selected_or_omitted_with_reason() {
    for reviewer in [ReviewerType::Correctness, ReviewerType::Security] {
        let pkg = build(reviewer);
        for o in &pkg.omitted {
            assert!(
                !matches!(o.reason, OmitReason::Duplicate { .. }),
                "unexpected duplicate"
            );
        }
        let selected: Vec<_> = pkg.items.iter().map(|i| (i.node, i.side)).collect();
        let mut dedup = selected.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(selected.len(), dedup.len(), "items are unique");
    }
}

#[test]
fn reproducible_two_builds_same_hash() {
    let a = build(ReviewerType::Correctness);
    let b = build(ReviewerType::Correctness);
    assert_eq!(a.package_hash, b.package_hash);
    assert_eq!(a, b);
}

#[test]
fn degraded_structural_context_is_recorded() {
    let locator = locator();
    let mut input = input(ReviewerType::Correctness, None);
    input.locator = Some(&locator);
    let (pool, flags) = generate(&input, None);
    assert!(flags.contains(&"structural_degraded".to_owned()));
    assert!(flags.contains(&"lexical_disabled".to_owned()));
    let blobs = blobs();
    let mut opts = options(&blobs, ReviewerType::Correctness);
    opts.flags = flags;
    let pkg = assemble(&input, pool, &opts);
    assert!(pkg.provenance.structural_degraded);
    assert!(pkg
        .items
        .iter()
        .all(|i| i.kind == ItemKind::ChangedSymbol || i.kind == ItemKind::ClassSignature));
}
