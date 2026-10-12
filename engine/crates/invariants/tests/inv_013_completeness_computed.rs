//! INV-013: completeness is computed, not self-reported.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on REV-001: subject not implemented on main"]
fn inv_013_reviewer_output_schema_has_no_completeness_fields() {
    unimplemented!("REV-001");
}

#[test]
#[ignore = "blocked on PIPE-008: subject not implemented on main"]
fn inv_013_model_claim_of_completeness_does_not_change_coverage() {
    unimplemented!("PIPE-008");
}

#[test]
#[ignore = "blocked on PIPE-008: subject not implemented on main"]
fn inv_013_coverage_inputs_are_persisted_state_only() {
    unimplemented!("PIPE-008");
}

#[test]
#[ignore = "blocked on PIPE-008, GW-009: subject not implemented on main"]
fn inv_013_random_model_text_mutations_never_change_coverage() {
    unimplemented!("PIPE-008, GW-009");
}

#[test]
#[ignore = "blocked on PIPE-008, GW-009: subject not implemented on main"]
fn inv_013_prompt_injected_completion_text_ignored() {
    unimplemented!("PIPE-008, GW-009");
}
