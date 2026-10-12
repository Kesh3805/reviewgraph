//! INV-012: failure is never approval; never auto-approve.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on PIPE-003, PIPE-007: subject not implemented on main"]
fn inv_012_every_stage_failure_ends_failed_and_publishes_nothing() {
    unimplemented!("PIPE-003, PIPE-007");
}

#[test]
#[ignore = "blocked on PIPE-003, PIPE-008: subject not implemented on main"]
fn inv_012_failed_and_terminal_runs_never_enqueue_publish() {
    unimplemented!("PIPE-003, PIPE-008");
}

#[test]
#[ignore = "blocked on PIPE-008: subject not implemented on main"]
fn inv_012_degraded_zero_findings_is_neutral() {
    unimplemented!("PIPE-008");
}

#[test]
#[ignore = "blocked on POL-001: subject not implemented on main"]
fn inv_012_config_rejects_auto_approve_keys() {
    unimplemented!("POL-001");
}
