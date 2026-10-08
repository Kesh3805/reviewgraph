//! Lineage consumption: persistence port and chained lookup (SID-005).
//!
//! `symbol_lineage` is owned by graph-storage; this module owns the type the caller hands to it and
//! the chain walk that finding history and Qdrant re-keying need. Both consumers ask the same
//! question — "this key used to be that key" — so it lives here.

use review_core::ids::SymbolKey;
use review_core::lineage::{
    follow_chain, AmbiguityNote, LineageIndex, LineageRecord, MatchRule, SymbolLineageRow,
    SymbolTransition,
};

/// Where lineage records go. Graph-storage implements it over the `symbol_lineage` table; a worker
/// can implement it over a queue or discard records in a dry run.
pub trait LineageSink {
    /// Records returned by the error type of the implementation.
    type Error;

    /// Persists one batch of records. Implementations must be idempotent per
    /// `(repository_id, from_snapshot_id, to_snapshot_id, from_key, to_key)`.
    fn write(&mut self, rows: &[SymbolLineageRow]) -> Result<(), Self::Error>;
}

/// What one matcher run produced, ready to be written or inspected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineageOutcome {
    /// The records, sorted deterministically.
    pub records: Vec<LineageRecord>,
    /// Added symbols with no counterpart.
    pub unmatched_added: Vec<SymbolKey>,
    /// Removed symbols with no counterpart.
    pub unmatched_removed: Vec<SymbolKey>,
    /// Symbols left unmatched because two candidates were equally plausible.
    pub ambiguous: Vec<AmbiguityNote>,
    /// The run fell back to rule 1 because the pools were over capacity.
    pub degraded: bool,
}

impl LineageOutcome {
    /// The rows to persist for one repository and snapshot pair.
    pub fn rows(
        &self,
        repository_id: uuid::Uuid,
        from_snapshot_id: uuid::Uuid,
        to_snapshot_id: uuid::Uuid,
    ) -> Vec<SymbolLineageRow> {
        self.records
            .iter()
            .map(|record| record.to_row(repository_id, from_snapshot_id, to_snapshot_id))
            .collect()
    }

    /// A chain index over the records.
    pub fn index(&self) -> LineageIndex {
        LineageIndex::from_records(&self.records)
    }

    /// Counts for the ADR-004 counters.
    pub fn counts(&self) -> LineageCounts {
        let mut counts = LineageCounts::default();
        for record in &self.records {
            match record.transition {
                SymbolTransition::Renamed => counts.renamed += 1,
                SymbolTransition::Moved => counts.moved += 1,
                SymbolTransition::RenamedAndMoved => counts.renamed_and_moved += 1,
            }
            match record.rule {
                MatchRule::ExactBody => counts.exact_body += 1,
                MatchRule::SignatureAndName => counts.signature_and_name += 1,
                MatchRule::TokenSimilarity => counts.token_similarity += 1,
            }
        }
        counts.ambiguous = self.ambiguous.len() as u32;
        counts
    }
}

/// Aggregated lineage counters (ADR-004).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineageCounts {
    /// Pure renames.
    pub renamed: u32,
    /// Pure moves.
    pub moved: u32,
    /// Renames that also changed the file.
    pub renamed_and_moved: u32,
    /// Pairs found by identical body.
    pub exact_body: u32,
    /// Pairs found by identical signature and name.
    pub signature_and_name: u32,
    /// Pairs found by token similarity.
    pub token_similarity: u32,
    /// Symbols left unmatched because candidates tied.
    pub ambiguous: u32,
}

/// Follows a chain of lineage records to its end, with the hop guard.
pub fn follow(index: &LineageIndex, key: SymbolKey) -> SymbolKey {
    index.follow(key)
}

/// Every key a chain passes through, with the hop guard.
pub fn chain(index: &LineageIndex, key: SymbolKey) -> Vec<SymbolKey> {
    follow_chain(index, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_core::ids::SymbolId;

    fn record(
        from: &str,
        to: &str,
        rule: MatchRule,
        transition: SymbolTransition,
    ) -> LineageRecord {
        let from_id = SymbolId::parse(from).unwrap();
        let to_id = SymbolId::parse(to).unwrap();
        LineageRecord {
            from: SymbolKey::of(&from_id),
            to: SymbolKey::of(&to_id),
            from_id,
            to_id,
            transition,
            rule,
            similarity: 1.0,
            ambiguous: false,
        }
    }

    #[derive(Debug)]
    struct Failing;

    #[test]
    fn outcomes_expose_rows_counts_and_chains() {
        let outcome = LineageOutcome {
            records: vec![
                record(
                    "ts:src/a#f/function",
                    "ts:src/a#g/function",
                    MatchRule::ExactBody,
                    SymbolTransition::Renamed,
                ),
                record(
                    "ts:src/a#g/function",
                    "ts:src/b#g/function",
                    MatchRule::SignatureAndName,
                    SymbolTransition::Moved,
                ),
            ],
            unmatched_added: vec![],
            unmatched_removed: vec![],
            ambiguous: vec![],
            degraded: false,
        };
        let counts = outcome.counts();
        assert_eq!(counts.renamed, 1);
        assert_eq!(counts.moved, 1);
        assert_eq!(counts.exact_body, 1);
        assert_eq!(counts.signature_and_name, 1);
        let rows = outcome.rows(
            uuid::Uuid::from_u128(7),
            uuid::Uuid::from_u128(8),
            uuid::Uuid::from_u128(9),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].repository_id, uuid::Uuid::from_u128(7));
        assert_eq!(rows[0].transition, "renamed");
        assert_eq!(rows[1].transition, "moved");
        let index = outcome.index();
        let start = SymbolKey::of(&SymbolId::parse("ts:src/a#f/function").unwrap());
        assert_eq!(follow(&index, start).to_string(), rows[1].to_key);
        assert_eq!(chain(&index, start).len(), 3);
    }

    #[test]
    fn a_sink_sees_the_rows_it_was_given() {
        struct Collecting(Vec<SymbolLineageRow>);
        impl LineageSink for Collecting {
            type Error = Failing;
            fn write(&mut self, rows: &[SymbolLineageRow]) -> Result<(), Failing> {
                self.0.extend_from_slice(rows);
                Ok(())
            }
        }
        let mut sink = Collecting(Vec::new());
        let rows = vec![SymbolLineageRow {
            repository_id: uuid::Uuid::from_u128(1),
            from_snapshot_id: uuid::Uuid::from_u128(2),
            to_snapshot_id: uuid::Uuid::from_u128(3),
            from_key: "0".repeat(32),
            to_key: "1".repeat(32),
            transition: "moved".to_owned(),
            similarity: 0.9,
            detail: Default::default(),
        }];
        assert!(sink.write(&rows).is_ok());
        assert_eq!(sink.0.len(), 1);
        assert_eq!(LineageCounts::default().renamed, 0);
    }
}
