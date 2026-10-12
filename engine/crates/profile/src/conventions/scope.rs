//! Scope narrowing (PROF-003): the broadest glob over which a convention holds with
//! consistency ≥ 0.9.
//!
//! Candidates are the whole repository (`**`) and every directory prefix of a sample's path up
//! to three levels deep (`src/**`, `src/modules/**`, `src/modules/users/**`). The chosen scope is
//! the candidate with the most samples among those at or above the threshold that hold at least
//! half of all samples (ties: the shorter glob, then the lexicographically smaller one). When none
//! qualifies the scope is `**`, and the convention is informational.

use std::collections::BTreeMap;

use super::confidence::{consistency, SCOPE_CONSISTENCY};

/// The chosen scope and the counts inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeChoice {
    pub glob: String,
    /// The directory prefix (`""` for the whole repository).
    pub prefix: String,
    pub conforming: u32,
    pub violating: u32,
}

impl ScopeChoice {
    pub fn contains(&self, path: &str) -> bool {
        in_prefix(&self.prefix, path)
    }
}

fn in_prefix(prefix: &str, path: &str) -> bool {
    prefix.is_empty() || path.starts_with(&format!("{prefix}/"))
}

/// Picks the scope for samples given as `(path, conforms)`.
pub fn narrow_scope(samples: &[(&str, bool)]) -> ScopeChoice {
    let mut counts: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for (path, conforms) in samples {
        let segments: Vec<&str> = path.split('/').collect();
        let dirs = segments.len().saturating_sub(1).min(3);
        let mut prefixes = vec![String::new()];
        for depth in 1..=dirs {
            prefixes.push(segments[..depth].join("/"));
        }
        for prefix in prefixes {
            let slot = counts.entry(prefix).or_default();
            if *conforms {
                slot.0 += 1;
            } else {
                slot.1 += 1;
            }
        }
    }
    // A scope must hold at least half of the samples: narrowing may drop a minority area where
    // the pattern does not hold, never the majority of the evidence.
    let total = counts.get("").map_or(0, |(c, v)| c + v);
    let min_samples = total.div_ceil(2).max(1);
    let best = counts
        .iter()
        .filter(|(_, (c, v))| c + v >= min_samples && consistency(*c, *v) >= SCOPE_CONSISTENCY)
        .max_by(|(pa, (ca, va)), (pb, (cb, vb))| {
            (ca + va)
                .cmp(&(cb + vb))
                .then(pb.len().cmp(&pa.len()))
                .then(pb.cmp(pa))
        });
    let (prefix, (conforming, violating)) = match best {
        Some((prefix, counts)) => (prefix.clone(), *counts),
        None => (
            String::new(),
            counts.get("").copied().unwrap_or_default(),
        ),
    };
    ScopeChoice {
        glob: if prefix.is_empty() {
            "**".to_owned()
        } else {
            format!("{prefix}/**")
        },
        prefix,
        conforming,
        violating,
    }
}
