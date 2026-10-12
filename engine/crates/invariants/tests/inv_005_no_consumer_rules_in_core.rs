//! INV-005 (PRD section 147, invariant 5): consumer-specific rules never become generic graph
//! semantics. New graph kinds must be generic (justified by two independent consumers).

mod common;

use common::{engine_root, read_allowlist, scan_crates, TestResult};

const GENERIC_CRATES: &[&str] = &[
    "codegraph",
    "analysis-ir",
    "incremental",
    "graph-storage",
    "diff-engine",
    "impact",
    "context-engine",
    "verification",
];

/// Framework tokens that may only appear under `lang-typescript/src/frameworks/`.
const FRAMEWORK_TOKENS: &[&str] = &["nestjs", "typeorm", "bullmq", "jest"];

#[test]
fn inv_005_framework_tokens_only_in_lang_typescript_frameworks() -> TestResult {
    let allowed: Vec<String> = read_allowlist("framework-tokens.txt")?
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let hits = scan_crates(GENERIC_CRATES, FRAMEWORK_TOKENS, &allowed)?;
    assert!(
        hits.is_empty(),
        "framework tokens in generic crates (allowlists/framework-tokens.txt needs a reason): {hits:#?}"
    );
    Ok(())
}

#[test]
fn inv_005_framework_allowlist_entries_have_reasons() -> TestResult {
    for (path, reason) in read_allowlist("framework-tokens.txt")? {
        assert!(!reason.is_empty(), "allowlist entry {path} has no reason");
        assert!(
            engine_root().join(&path).is_file(),
            "allowlist entry {path} points at a missing file"
        );
    }
    Ok(())
}

#[test]
#[ignore = "blocked on POL-007 (reference profile): consumer denylist terms are recorded there"]
fn inv_005_generic_crates_contain_no_consumer_terms() {
    unimplemented!("POL-007");
}

#[test]
#[ignore = "blocked on POL-007: the reference rule pack location is defined there"]
fn inv_005_rule_pack_lives_outside_engine_crates() {
    unimplemented!("POL-007");
}

#[test]
#[ignore = "blocked on the insta snapshot of NodeKind/EdgeKind lists (CG-001/CG-002 follow-up): needs cargo-insta run in CI"]
fn inv_005_graph_kind_lists_match_snapshot() {
    unimplemented!("CG-001, CG-002");
}
