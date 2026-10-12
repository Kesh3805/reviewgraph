//! INV-010: comment quantity is never an optimization target.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on DED-004, VER-010: subject not implemented on main"]
fn inv_010_decision_functions_take_no_count_arguments() {
    unimplemented!("DED-004, VER-010");
}

#[test]
#[ignore = "blocked on VER-010: subject not implemented on main"]
fn inv_010_finding_decisions_independent_of_siblings() {
    unimplemented!("VER-010");
}

#[test]
#[ignore = "blocked on POL-001: subject not implemented on main"]
fn inv_010_config_rejects_quantity_target_keys() {
    unimplemented!("POL-001");
}

#[test]
#[ignore = "blocked on GH-007: subject not implemented on main"]
fn inv_010_comment_cap_overflows_to_summary_never_drops() {
    unimplemented!("GH-007");
}

#[test]
#[ignore = "blocked on QB-001: subject not implemented on main"]
fn inv_010_quality_gate_has_no_comment_volume_term() {
    unimplemented!("QB-001");
}

#[test]
#[ignore = "blocked on PIPE-003, GH-007: subject not implemented on main"]
fn inv_010_zero_findings_review_is_valid_and_publishes_summary_only() {
    unimplemented!("PIPE-003, GH-007");
}
