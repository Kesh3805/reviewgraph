//! INV-015: out-of-diff findings are relocated to the summary, never dropped.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on GH-007: subject not implemented on main"]
fn inv_015_placement_partitions_publishable_findings() {
    unimplemented!("GH-007");
}

#[test]
#[ignore = "blocked on GH-007, VER-010: subject not implemented on main"]
fn inv_015_out_of_diff_finding_appears_in_summary_not_comments() {
    unimplemented!("GH-007, VER-010");
}

#[test]
#[ignore = "blocked on GH-007: subject not implemented on main"]
fn inv_015_comment_cap_overflow_goes_to_summary() {
    unimplemented!("GH-007");
}
