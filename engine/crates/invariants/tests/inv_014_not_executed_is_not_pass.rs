//! INV-014: NOT_EXECUTED is never PASS.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on PIPE-004: subject not implemented on main"]
fn inv_014_unrunnable_tools_are_not_executed_with_reason() {
    unimplemented!("PIPE-004");
}

#[test]
#[ignore = "blocked on VER-005: subject not implemented on main"]
fn inv_014_not_executed_has_no_conversion_to_passed() {
    unimplemented!("VER-005");
}

#[test]
#[ignore = "blocked on VER-005, VER-009: subject not implemented on main"]
fn inv_014_not_executed_blocking_tool_does_not_block() {
    unimplemented!("VER-005, VER-009");
}

#[test]
#[ignore = "blocked on VER-009: subject not implemented on main"]
fn inv_014_not_executed_gives_no_deterministic_signal() {
    unimplemented!("VER-009");
}
