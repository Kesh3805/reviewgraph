//! Rename/move lineage records (ADR-005, SID-005).
//!
//! A rename or a move changes a `SymbolId`, so the removed and added symbols are paired and the
//! pair is recorded as a lineage record. Finding history and embeddings follow the chain; the rows
//! themselves are persisted by graph-storage.

use std::collections::BTreeMap;

use crate::ids::{SymbolId, SymbolKey};
use serde::{Deserialize, Serialize};

/// How a symbol changed identity between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolTransition {
    /// Same module path, different name.
    #[serde(rename = "renamed")]
    Renamed,
    /// Same name, different module path.
    #[serde(rename = "moved")]
    Moved,
    /// Different name and different module path.
    #[serde(rename = "renamed_moved")]
    RenamedAndMoved,
}

impl SymbolTransition {
    /// The persisted, stable wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Renamed => "renamed",
            Self::Moved => "moved",
            Self::RenamedAndMoved => "renamed_moved",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Option<Self> {
        match text {
            "renamed" => Some(Self::Renamed),
            "moved" => Some(Self::Moved),
            "renamed_moved" => Some(Self::RenamedAndMoved),
            _ => None,
        }
    }
}

/// Which rule of the ladder paired two symbols (ADR-005). The wire name is persisted, so it is
/// part of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MatchRule {
    /// Identical `body_hash` with enough tokens to be meaningful.
    #[serde(rename = "exact_body")]
    ExactBody,
    /// Identical `signature_hash` and the same simple name in a different file.
    #[serde(rename = "signature_and_name")]
    SignatureAndName,
    /// Token Jaccard similarity at or above the configured threshold.
    #[serde(rename = "token_similarity")]
    TokenSimilarity,
}

impl MatchRule {
    /// The persisted, stable wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactBody => "exact_body",
            Self::SignatureAndName => "signature_and_name",
            Self::TokenSimilarity => "token_similarity",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Option<Self> {
        match text {
            "exact_body" => Some(Self::ExactBody),
            "signature_and_name" => Some(Self::SignatureAndName),
            "token_similarity" => Some(Self::TokenSimilarity),
            _ => None,
        }
    }

    /// Rules are tried in this order.
    pub const ORDER: [MatchRule; 3] = [
        MatchRule::ExactBody,
        MatchRule::SignatureAndName,
        MatchRule::TokenSimilarity,
    ];
}

/// One rename or move: `from_key` is the symbol as it existed in the base snapshot, `to_key` the
/// same symbol in the head snapshot. Rows of `symbol_lineage` are built from these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineageRecord {
    /// Storage key of the symbol in the base snapshot.
    pub from: SymbolKey,
    /// Storage key of the symbol in the head snapshot.
    pub to: SymbolKey,
    /// Canonical id in the base snapshot.
    pub from_id: SymbolId,
    /// Canonical id in the head snapshot.
    pub to_id: SymbolId,
    /// How the identity changed.
    pub transition: SymbolTransition,
    /// Which rule paired them.
    pub rule: MatchRule,
    /// Confidence in `[0, 1]`; `1.0` for an exact body match.
    pub similarity: f32,
    /// True when another candidate was within `0.02` of the winner.
    pub ambiguous: bool,
}

impl LineageRecord {
    /// The persisted row of `symbol_lineage` (the table itself is owned by graph-storage).
    ///
    /// `repository_id` and the snapshot ids are supplied by the caller: lineage is scoped to one
    /// repository and one snapshot pair, and neither is part of the record.
    pub fn to_row(
        &self,
        repository_id: uuid::Uuid,
        from_snapshot_id: uuid::Uuid,
        to_snapshot_id: uuid::Uuid,
    ) -> SymbolLineageRow {
        SymbolLineageRow {
            repository_id,
            from_snapshot_id,
            to_snapshot_id,
            from_key: self.from.to_string(),
            to_key: self.to.to_string(),
            transition: self.transition.as_str().to_owned(),
            similarity: self.similarity,
            detail: BTreeMap::from([
                ("from_id".to_owned(), self.from_id.as_str().to_owned()),
                ("to_id".to_owned(), self.to_id.as_str().to_owned()),
                ("rule".to_owned(), self.rule.as_str().to_owned()),
                ("ambiguous".to_owned(), self.ambiguous.to_string()),
            ]),
        }
    }
}

/// One row of `symbol_lineage`, as graph-storage persists it. `rule` and `ambiguous` live in the
/// `detail` column, because the table predates them (ADR-014).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolLineageRow {
    /// Tenant and repository the lineage belongs to.
    pub repository_id: uuid::Uuid,
    /// Snapshot the symbol disappeared from.
    pub from_snapshot_id: uuid::Uuid,
    /// Snapshot the symbol appeared in.
    pub to_snapshot_id: uuid::Uuid,
    /// `SymbolKey` of the removed symbol, 32 lowercase hex characters.
    pub from_key: String,
    /// `SymbolKey` of the added symbol, 32 lowercase hex characters.
    pub to_key: String,
    /// `renamed | moved | renamed_moved`.
    pub transition: String,
    /// Confidence in `[0, 1]`.
    pub similarity: f32,
    /// `from_id`, `to_id`, `rule` and `ambiguous`, as a `jsonb` object.
    pub detail: BTreeMap<String, String>,
}

/// A symbol that had several equally plausible predecessors, so nothing was paired. A wrong
/// pairing is worse than none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguityNote {
    /// The added symbol that stayed unmatched.
    pub key: SymbolKey,
    /// Its canonical id.
    pub id: SymbolId,
    /// The removed symbols that were equally plausible.
    pub candidates: Vec<SymbolKey>,
    /// Why the decision was not made.
    pub reason: AmbiguityReason,
}

/// Why an ambiguity was left unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityReason {
    /// Several candidates were identical after every tie-breaker.
    EqualCandidates,
    /// The winner beat the runner-up by less than the ambiguity margin.
    CloseMatch,
}

/// Maximum number of hops [`follow`] walks before it gives up on a cyclic chain.
pub const MAX_FOLLOW_HOPS: usize = 32;

/// Chained lineage records, keyed by the key they replace.
#[derive(Debug, Clone, Default)]
pub struct LineageIndex {
    records: BTreeMap<(String, String), LineageRecord>,
}

impl LineageIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one record. A repeated `from` keeps the first record, so a later re-run cannot
    /// rewrite history.
    pub fn push(&mut self, record: LineageRecord) {
        let key = (record.from.to_string(), record.to.to_string());
        self.records.entry(key).or_insert(record);
    }

    /// Builds an index from a slice of records.
    pub fn from_records(records: &[LineageRecord]) -> Self {
        let mut index = Self::new();
        for record in records {
            index.push(record.clone());
        }
        index
    }

    /// Every record, sorted by `from` then `to`.
    pub fn records(&self) -> impl Iterator<Item = &LineageRecord> {
        self.records.values()
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the index holds no record.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The key `key` became, if any.
    pub fn successor(&self, key: SymbolKey) -> Option<SymbolKey> {
        self.records
            .values()
            .find(|record| record.from == key)
            .map(|record| record.to)
    }

    /// The key `key` came from, if any.
    pub fn predecessor(&self, key: SymbolKey) -> Option<SymbolKey> {
        self.records
            .values()
            .find(|record| record.to == key)
            .map(|record| record.from)
    }

    /// Follows a chain of records to its end, stopping after [`MAX_FOLLOW_HOPS`] hops so a cyclic or
    /// self-referential row cannot hang a review.
    pub fn follow(&self, key: SymbolKey) -> SymbolKey {
        let mut current = key;
        for _ in 0..MAX_FOLLOW_HOPS {
            match self.successor(current) {
                Some(next) if next != current => current = next,
                _ => return current,
            }
        }
        current
    }
}

/// Walks a chain of lineage records from `key`, returning every key it passed through. Stops after
/// [`MAX_FOLLOW_HOPS`] hops.
pub fn follow_chain(index: &LineageIndex, key: SymbolKey) -> Vec<SymbolKey> {
    let mut chain = vec![key];
    let mut current = key;
    for _ in 0..MAX_FOLLOW_HOPS {
        match index.successor(current) {
            Some(next) if next != current => {
                chain.push(next);
                current = next;
            }
            _ => break,
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(from: &str, to: &str, rule: MatchRule) -> LineageRecord {
        let from_id = SymbolId::parse(from).unwrap();
        let to_id = SymbolId::parse(to).unwrap();
        LineageRecord {
            from: SymbolKey::of(&from_id),
            to: SymbolKey::of(&to_id),
            from_id,
            to_id,
            transition: SymbolTransition::Renamed,
            rule,
            similarity: 1.0,
            ambiguous: false,
        }
    }

    #[test]
    fn wire_names_are_stable() {
        for transition in [
            SymbolTransition::Renamed,
            SymbolTransition::Moved,
            SymbolTransition::RenamedAndMoved,
        ] {
            assert_eq!(
                transition.as_str(),
                serde_json::to_string(&transition)
                    .unwrap()
                    .trim_matches('"')
            );
            assert_eq!(
                SymbolTransition::from_str(transition.as_str()),
                Some(transition)
            );
        }
        for rule in MatchRule::ORDER {
            assert_eq!(
                rule.as_str(),
                serde_json::to_string(&rule).unwrap().trim_matches('"')
            );
            assert_eq!(MatchRule::from_str(rule.as_str()), Some(rule));
        }
        assert!(SymbolTransition::from_str("nope").is_none());
        assert!(MatchRule::from_str("nope").is_none());
    }

    #[test]
    fn records_chain_and_stop_on_cycles() {
        let a = record(
            "ts:src/a#f/function",
            "ts:src/a#g/function",
            MatchRule::ExactBody,
        );
        let b = record(
            "ts:src/a#g/function",
            "ts:src/a#h/function",
            MatchRule::ExactBody,
        );
        let index = LineageIndex::from_records(&[a.clone(), b.clone()]);
        assert_eq!(index.len(), 2);
        assert!(!index.is_empty());
        assert_eq!(index.follow(a.from), b.to);
        assert_eq!(index.follow(a.to), b.to);
        assert_eq!(follow_chain(&index, a.from), vec![a.from, a.to, b.to]);
        assert_eq!(index.predecessor(b.to), Some(a.to));
        assert!(index.successor(b.to).is_none());
        assert_eq!(index.records().count(), 2);

        let mut cyclic = LineageIndex::new();
        cyclic.push(record(
            "ts:src/a#f/function",
            "ts:src/a#g/function",
            MatchRule::ExactBody,
        ));
        cyclic.push(record(
            "ts:src/a#g/function",
            "ts:src/a#f/function",
            MatchRule::ExactBody,
        ));
        assert!(follow_chain(&cyclic, a.from).len() <= MAX_FOLLOW_HOPS + 1);
        assert!(LineageIndex::new().is_empty());
    }

    #[test]
    fn rows_carry_the_detail_column() {
        let record = record(
            "ts:src/a#f/function",
            "ts:src/a#g/function",
            MatchRule::TokenSimilarity,
        );
        let row = record.to_row(
            uuid::Uuid::from_u128(1),
            uuid::Uuid::from_u128(2),
            uuid::Uuid::from_u128(3),
        );
        assert_eq!(row.transition, "renamed");
        assert_eq!(row.detail["rule"], "token_similarity");
        assert_eq!(row.detail["ambiguous"], "false");
        assert_eq!(row.detail["from_id"], record.from_id.as_str());
        assert_eq!(row.from_key.len(), 32);
        assert_eq!(row.to_key.len(), 32);
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["transition"], "renamed");
        assert_eq!(json["detail"]["rule"], "token_similarity");
    }
}
