//! Counters the incremental stage aggregates and exports (ADR-004).
//!
//! The struct is the shared shape; the telemetry crate fills it in when the indexer runs. Keeping it
//! here means the differ, the matcher and the tests agree on the field names.

/// Symbols added, removed and modified, plus how the matcher behaved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IncrementalCounters {
    /// Symbols that only exist in the head unit.
    pub symbols_added: u64,
    /// Symbols that only exist in the base unit.
    pub symbols_removed: u64,
    /// Symbols with at least one changed hash.
    pub symbols_modified: u64,
    /// Symbols whose signature changed.
    pub symbols_modified_signature: u64,
    /// Symbols whose body changed.
    pub symbols_modified_body: u64,
    /// Symbols with a changed decorator or export flag.
    pub symbols_modified_attributes: u64,
    /// Files whose diff could not be computed.
    pub files_unknown: u64,
    /// Symbols flagged `uncertain` because of a parse error.
    pub symbols_uncertain: u64,
    /// Renames found by the matcher.
    pub symbols_renamed: u64,
    /// Moves found by the matcher.
    pub symbols_moved: u64,
    /// Matches found by rule 1.
    pub matches_exact_body: u64,
    /// Matches found by rule 2.
    pub matches_signature_and_name: u64,
    /// Matches found by rule 3.
    pub matches_token_similarity: u64,
    /// Symbols left unmatched because candidates tied.
    pub matcher_ambiguous: u64,
    /// Matcher runs that fell back to rule 1.
    pub matcher_degraded: u64,
}

impl IncrementalCounters {
    /// Adds one file's counts.
    pub fn add_diff(&mut self, counts: crate::symbol_diff::DiffCounts) {
        self.symbols_added += u64::from(counts.added);
        self.symbols_removed += u64::from(counts.removed);
        self.symbols_modified += u64::from(
            counts.modified_signature + counts.modified_body + counts.modified_attributes,
        );
        self.symbols_modified_signature += u64::from(counts.modified_signature);
        self.symbols_modified_body += u64::from(counts.modified_body);
        self.symbols_modified_attributes += u64::from(counts.modified_attributes);
    }

    /// Adds one matcher run's lineage counts.
    pub fn add_lineage(&mut self, counts: crate::lineage::LineageCounts) {
        self.symbols_renamed += u64::from(counts.renamed + counts.renamed_and_moved);
        self.symbols_moved += u64::from(counts.moved + counts.renamed_and_moved);
        self.matches_exact_body += u64::from(counts.exact_body);
        self.matches_signature_and_name += u64::from(counts.signature_and_name);
        self.matches_token_similarity += u64::from(counts.token_similarity);
        self.matcher_ambiguous += u64::from(counts.ambiguous);
    }

    /// Every counter at zero.
    pub fn zeroed() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lineage::LineageCounts;
    use crate::symbol_diff::DiffCounts;

    #[test]
    fn counters_accumulate() {
        let mut counters = IncrementalCounters::zeroed();
        counters.add_diff(DiffCounts {
            unchanged: 5,
            modified_signature: 1,
            modified_body: 2,
            modified_attributes: 3,
            added: 4,
            removed: 6,
        });
        counters.add_lineage(LineageCounts {
            renamed: 1,
            moved: 2,
            renamed_and_moved: 1,
            exact_body: 3,
            signature_and_name: 1,
            token_similarity: 0,
            ambiguous: 1,
        });
        assert_eq!(counters.symbols_added, 4);
        assert_eq!(counters.symbols_removed, 6);
        assert_eq!(counters.symbols_modified, 6);
        assert_eq!(
            counters.symbols_renamed, 2,
            "renamed plus renamed-and-moved"
        );
        assert_eq!(counters.symbols_moved, 3);
        assert_eq!(counters.matcher_ambiguous, 1);
        assert_eq!(
            IncrementalCounters::default(),
            IncrementalCounters::zeroed()
        );
    }
}
