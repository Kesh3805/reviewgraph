//! INV-001 (PRD section 147, invariant 1): PR review never requires a complete graph rebuild.
//!
//! Only the static guard can run today. The counter-based tests need the review pipeline
//! (PIPE-003) and the incremental snapshot writer (INC-009); they are `#[ignore]`d until then.

mod common;

use common::{crate_src, rel, rust_files, TestResult};
use std::fs;

/// The full-index entry point. Only the `repository-index` job handler may call it.
const FULL_INDEX_ENTRY: &str = "FullIndexer";

#[test]
fn inv_001_pr_path_has_no_reference_to_full_indexer() -> TestResult {
    let mut offenders = Vec::new();
    for file in rust_files(&crate_src("pipeline"))? {
        if fs::read_to_string(&file)?.contains(FULL_INDEX_ENTRY) {
            offenders.push(rel(&file));
        }
    }
    assert!(
        offenders.is_empty(),
        "the pipeline crate must not reference {FULL_INDEX_ENTRY}; the only allowed caller is the repository-index handler: {offenders:?}"
    );
    Ok(())
}

#[test]
#[ignore = "blocked on PIPE-003 (review pipeline) and INC-009 (delta snapshots): counter graph_full_rebuilds_total does not exist yet"]
fn inv_001_pr_review_does_not_rebuild_graph() {
    unimplemented!("PIPE-003, INC-009");
}

#[test]
#[ignore = "blocked on PIPE-003 and INC-009: files_reparsed_total is not emitted by a review run yet"]
fn inv_001_changed_files_only_are_reparsed() {
    unimplemented!("PIPE-003, INC-009");
}

#[test]
#[ignore = "blocked on PIPE-003: the base_snapshot_missing transient failure path is not implemented"]
fn inv_001_missing_base_snapshot_does_not_trigger_inline_rebuild() {
    unimplemented!("PIPE-003");
}
