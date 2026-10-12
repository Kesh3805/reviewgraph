//! INV-002: LLM output never directly published.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on VER-001, VER-010: subject not implemented on main"]
fn inv_002_raw_candidate_cannot_construct_published_finding() {
    unimplemented!("VER-001, VER-010");
}

#[test]
#[ignore = "blocked on VER-010, PIPE-003: subject not implemented on main"]
fn inv_002_fabricated_candidate_is_suppressed_and_not_published() {
    unimplemented!("VER-010, PIPE-003");
}

#[test]
#[ignore = "blocked on VER-010, GH-009: subject not implemented on main"]
fn inv_002_every_published_row_has_verification_record() {
    unimplemented!("VER-010, GH-009");
}
