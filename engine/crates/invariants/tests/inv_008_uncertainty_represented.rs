//! INV-008: graph uncertainty is represented.
//!
//! Subject code is not implemented on main yet, so each test is `#[ignore]`d with the
//! blocking task IDs. Remove the attribute and write the body when the task lands.

#[test]
#[ignore = "blocked on CG-005: subject not implemented on main"]
fn inv_008_ambiguous_call_keeps_low_confidence_edges() {
    unimplemented!("CG-005");
}

#[test]
#[ignore = "blocked on CG-005: subject not implemented on main"]
fn inv_008_every_edge_has_confidence_resolved_by_and_provenance() {
    unimplemented!("CG-005");
}

#[test]
#[ignore = "blocked on CTX-001: subject not implemented on main"]
fn inv_008_context_items_carry_edge_confidence() {
    unimplemented!("CTX-001");
}

#[test]
#[ignore = "blocked on VER-005, VER-009: subject not implemented on main"]
fn inv_008_low_confidence_relation_lowers_graph_signal_and_adds_uncertainty() {
    unimplemented!("VER-005, VER-009");
}

#[test]
#[ignore = "blocked on API-011: subject not implemented on main"]
fn inv_008_graph_api_edge_schema_requires_confidence() {
    unimplemented!("API-011");
}
