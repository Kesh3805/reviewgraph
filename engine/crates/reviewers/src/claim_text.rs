//! Claim normalisation for root-cause fingerprints (REV-C-003, consumed by DED-001).
//!
//! Lowercase; refs replaced by their target ids (ids keep their spelling); markdown and
//! punctuation stripped; hedge words dropped; whitespace collapsed.

use crate::refs::{RefKind, RefTable};

/// Words that carry no claim content.
pub const HEDGE_WORDS: &[&str] = &[
    "might",
    "could",
    "potentially",
    "possibly",
    "perhaps",
    "maybe",
];

/// Normalises `claim` against `refs`.
pub fn normalize(claim: &str, refs: &RefTable) -> String {
    let mut out: Vec<String> = Vec::new();
    for raw in claim.split_whitespace() {
        let trimmed = raw.trim_matches(|c: char| !c.is_alphanumeric());
        if RefKind::from_ref(trimmed).is_some() {
            if let Some(entry) = refs.get(trimmed) {
                out.push(entry.target.clone());
                continue;
            }
        }
        let word: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .flat_map(char::to_lowercase)
            .collect();
        if word.is_empty() || HEDGE_WORDS.contains(&word.as_str()) {
            continue;
        }
        out.push(word);
    }
    out.join(" ")
}
