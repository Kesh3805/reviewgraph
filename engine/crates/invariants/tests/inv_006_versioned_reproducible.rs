//! INV-006: intelligence is versioned and reproducible.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on PIPE-009, VER-012: subject not implemented on main"]
fn inv_006_all_derived_rows_carry_provenance() {
    unimplemented!("PIPE-009, VER-012");
}

#[test]
#[ignore = "blocked on IDX-002, CTX-009: subject not implemented on main"]
fn inv_006_cache_keys_include_their_versions() {
    unimplemented!("IDX-002, CTX-009");
}

#[test]
#[ignore = "blocked on IDX-002, CTX-009: subject not implemented on main"]
fn inv_006_version_bump_invalidates_only_dependent_layers() {
    unimplemented!("IDX-002, CTX-009");
}

#[test]
#[ignore = "blocked on PIPE-009: subject not implemented on main"]
fn inv_006_pipeline_is_reproducible_under_replay() {
    unimplemented!("PIPE-009");
}

#[test]
#[ignore = "blocked on PIPE-009: subject not implemented on main"]
fn inv_006_status_and_api_expose_fingerprint() {
    unimplemented!("PIPE-009");
}
