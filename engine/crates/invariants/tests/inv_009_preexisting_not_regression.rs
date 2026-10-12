//! INV-009: pre-existing defects are not reported as regressions.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on VER-006: subject not implemented on main"]
fn inv_009_unrelated_preexisting_defect_suppressed() {
    unimplemented!("VER-006");
}

#[test]
#[ignore = "blocked on VER-006: subject not implemented on main"]
fn inv_009_adjacent_edit_without_predicate_change_suppressed() {
    unimplemented!("VER-006");
}

#[test]
#[ignore = "blocked on VER-006, VER-010: subject not implemented on main"]
fn inv_009_exposure_expansion_published_with_accurate_class() {
    unimplemented!("VER-006, VER-010");
}

#[test]
#[ignore = "blocked on VER-006, VER-010: subject not implemented on main"]
fn inv_009_introduced_defect_published_as_introduced() {
    unimplemented!("VER-006, VER-010");
}
