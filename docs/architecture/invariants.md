# Invariants index

Executable guards for the product invariants (PRD section 147). Rust tests live in the dev-only
`engine/crates/invariants` crate (no library code); TypeScript tests live in
`apps/api/test/invariants/`. Tests marked `#[ignore]` (Rust) or `it.skip` (TS) name the task whose
subject code is missing; un-ignore them when that task lands.

Allowlists in `engine/crates/invariants/allowlists/` are `path | reason` files so additions are
visible in review.

| Invariant | Rust | TypeScript | State |
|---|---|---|---|
| INV-001 no full rebuild | `tests/inv_001_no_full_rebuild.rs` | | static guard live; counters ignored (PIPE-003, INC-009) |
| INV-002 LLM output not published | `tests/inv_002_llm_output_not_published.rs` | `inv-002-publisher-reads-verified-only.spec.ts` | ignored (VER-001, VER-010) |
| INV-003 findings anchored | `tests/inv_003_findings_anchored.rs` | | ignored (VER-004) |
| INV-004 no provider code in domain | `tests/inv_004_no_provider_code_in_domain.rs` | `inv-004-domain-imports-no-provider.spec.ts` | live |
| INV-005 no consumer rules in core | `tests/inv_005_no_consumer_rules_in_core.rs` | | framework containment live; denylist, rule pack and kind snapshot ignored (POL-007, CG-001) |
| INV-006 versioned and reproducible | `tests/inv_006_versioned_reproducible.rs` | | ignored (PIPE-009) |
| INV-007 budgeted context | `tests/inv_007_context_budget.rs` | | ignored (CTX-006, CTX-010) |
| INV-008 uncertainty represented | `tests/inv_008_uncertainty_represented.rs` | | ignored (CG-005) |
| INV-009 pre-existing not regression | `tests/inv_009_preexisting_not_regression.rs` | `inv-009-render-never-says-introduced-for-preexisting.spec.ts` | ignored (VER-006) |
| INV-010 no comment quantity target | `tests/inv_010_comment_quantity_not_optimized.rs` | `inv-010-config-has-no-quantity-targets.spec.ts` | ignored (POL-001, GH-007) |
| INV-011 never merge | `tests/inv_011_no_merge_capability.rs` | `no-merge.spec.ts` | live (E2E recorder case ignored, E2E-001) |
| INV-012 failure is never approval | `tests/inv_012_failure_never_approval.rs` | `inv-012-publisher-failure-matrix.spec.ts` | ignored (PIPE-003, PIPE-008) |
| INV-013 completeness computed | `tests/inv_013_completeness_computed.rs` | | ignored (PIPE-008) |
| INV-014 NOT_EXECUTED is not PASS | `tests/inv_014_not_executed_is_not_pass.rs` | `inv-014-summary-shows-not-executed.spec.ts` | ignored (VER-005) |
| INV-015 out-of-diff relocated | `tests/inv_015_out_of_diff_relocated.rs` | `inv-015-conservation.spec.ts` | ignored (GH-007) |
