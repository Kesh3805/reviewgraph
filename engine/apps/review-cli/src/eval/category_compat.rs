//! Closed, versioned category compatibility table for the matcher (EVAL-002).

/// Bump when the table below changes.
pub const MATCHER_VERSION: u32 = 1;

/// Unordered compatible pairs (a finding of one category may satisfy an expectation of the other).
const COMPATIBLE: &[(&str, &str)] = &[
    ("security", "correctness"),
    ("transaction", "correctness"),
    ("concurrency", "correctness"),
    ("api_contract", "correctness"),
    ("database_safety", "correctness"),
    ("test", "correctness"),
];

/// True when `expected` is `any`, equals `actual`, or the pair is in the compat table.
pub fn categories_compatible(expected: &str, actual: &str) -> bool {
    expected == "any"
        || expected == actual
        || COMPATIBLE
            .iter()
            .any(|(a, b)| (*a == expected && *b == actual) || (*a == actual && *b == expected))
}
