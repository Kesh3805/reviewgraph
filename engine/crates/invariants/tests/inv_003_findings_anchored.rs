//! INV-003: every published finding is anchored.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on VER-004: subject not implemented on main"]
fn inv_003_in_hunk_finding_is_anchored_and_publishable() {
    unimplemented!("VER-004");
}

#[test]
#[ignore = "blocked on VER-004, IMP-008: subject not implemented on main"]
fn inv_003_impact_path_finding_records_path_from_changed_symbol() {
    unimplemented!("VER-004, IMP-008");
}

#[test]
#[ignore = "blocked on VER-004: subject not implemented on main"]
fn inv_003_unrelated_file_finding_suppressed() {
    unimplemented!("VER-004");
}

#[test]
#[ignore = "blocked on VER-004: subject not implemented on main"]
fn inv_003_unchanged_region_without_path_suppressed() {
    unimplemented!("VER-004");
}

#[test]
#[ignore = "blocked on VER-004, VER-010: subject not implemented on main"]
fn inv_003_property_published_implies_anchor() {
    unimplemented!("VER-004, VER-010");
}

#[test]
#[ignore = "blocked on VER-010, PIPE-003: subject not implemented on main"]
fn inv_003_db_query_finds_no_unanchored_published_rows() {
    unimplemented!("VER-010, PIPE-003");
}
