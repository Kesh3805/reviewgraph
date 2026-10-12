//! Category compatibility for the matcher (EVAL-002). The table is closed and versioned with
//! [`MATCHER_VERSION`]; changing it changes every score, so it needs a version bump.
//!
//! | Expectation category | Also matches finding categories |
//! |---|---|
//! | `correctness` | `security`, `data_integrity`, `concurrency`, `error_handling`, `api_contract` |
//! | `security` | `correctness` |
//! | `transaction` | `correctness`, `data_integrity` |
//! | `concurrency` | `correctness` |
//! | `api_contract` | `correctness`, `architecture` |
//! | `error_handling` | `correctness` |
//! | `data_integrity` | `correctness` |
//! | `tests` | `testing` |
//!
//! `security ↔ correctness` is unconditional here: the produced finding does not carry its
//! predicate yet, so the "authorization and validation predicates only" refinement waits for VER.

/// Version of the matcher rules and this table.
pub const MATCHER_VERSION: &str = "1";

const TABLE: &[(&str, &[&str])] = &[
    (
        "correctness",
        &[
            "security",
            "data_integrity",
            "concurrency",
            "error_handling",
            "api_contract",
        ],
    ),
    ("security", &["correctness"]),
    ("transaction", &["correctness", "data_integrity"]),
    ("concurrency", &["correctness"]),
    ("api_contract", &["correctness", "architecture"]),
    ("error_handling", &["correctness"]),
    ("data_integrity", &["correctness"]),
    ("tests", &["testing"]),
];

/// Whether an expectation category accepts a finding category.
pub fn compatible(expected: &str, finding: &str) -> bool {
    if expected == "any" || expected == finding {
        return true;
    }
    TABLE
        .iter()
        .find(|(e, _)| *e == expected)
        .is_some_and(|(_, ok)| ok.contains(&finding))
}
