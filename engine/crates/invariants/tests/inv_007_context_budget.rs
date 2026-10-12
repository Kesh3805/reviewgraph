//! INV-007: context selection is explicitly budgeted.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on CTX-006, CTX-010: subject not implemented on main"]
fn inv_007_context_package_never_exceeds_budget() {
    unimplemented!("CTX-006, CTX-010");
}

#[test]
#[ignore = "blocked on CTX-010: subject not implemented on main"]
fn inv_007_omitted_items_always_have_reasons() {
    unimplemented!("CTX-010");
}

#[test]
#[ignore = "blocked on CG-006, IMP-007: subject not implemented on main"]
fn inv_007_graph_traversals_respect_node_and_depth_budgets() {
    unimplemented!("CG-006, IMP-007");
}

#[test]
#[ignore = "blocked on CTX-010: subject not implemented on main"]
fn inv_007_truncation_is_reported() {
    unimplemented!("CTX-010");
}

#[test]
#[ignore = "blocked on CTX-006: subject not implemented on main"]
fn inv_007_no_budgetless_constructors() {
    unimplemented!("CTX-006");
}

#[test]
#[ignore = "blocked on PIPE-006, GW-001: subject not implemented on main"]
fn inv_007_model_calls_capped_under_contention() {
    unimplemented!("PIPE-006, GW-001");
}

#[test]
#[ignore = "blocked on CTX-006: subject not implemented on main"]
fn inv_007_semantic_candidates_only_fill_remaining_budget() {
    unimplemented!("CTX-006");
}
