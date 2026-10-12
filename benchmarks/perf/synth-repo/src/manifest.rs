//! `manifest.json`: exact expected counts for a generated tree, so benches can assert that the
//! parser and the graph builder saw everything the generator planted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Version of the generator's output format. Part of the idempotency key.
pub const GENERATOR_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub files: u64,
    pub bytes: u64,
    /// Declared symbols by IR kind (the per-file module symbol is not counted).
    pub symbols_by_kind: BTreeMap<String, u64>,
    pub routes: u64,
    pub entities: u64,
    pub processors: u64,
    pub spec_files: u64,
    pub generated_files: u64,
    pub imports: u64,
    /// Calls through constructor-injected services of the same module (and own repository).
    pub calls_di: u64,
    /// Calls to functions imported from `src/shared`.
    pub calls_shared: u64,
    /// Calls into hub services of other modules (Zipf distributed).
    pub calls_cross_module: u64,
    /// Calls on an untyped receiver whose name resolves to several candidates.
    pub calls_ambiguous: u64,
    /// Planted call count per hub rank (rank 0 is the heaviest hub).
    pub hub_fanin: BTreeMap<u32, u64>,
}

impl Counts {
    pub fn sym(&mut self, kind: &str, n: u64) {
        *self.symbols_by_kind.entry(kind.to_owned()).or_insert(0) += n;
    }

    pub fn merge(&mut self, other: &Counts) {
        self.files += other.files;
        self.bytes += other.bytes;
        for (k, v) in &other.symbols_by_kind {
            *self.symbols_by_kind.entry(k.clone()).or_insert(0) += v;
        }
        self.routes += other.routes;
        self.entities += other.entities;
        self.processors += other.processors;
        self.spec_files += other.spec_files;
        self.generated_files += other.generated_files;
        self.imports += other.imports;
        self.calls_di += other.calls_di;
        self.calls_shared += other.calls_shared;
        self.calls_cross_module += other.calls_cross_module;
        self.calls_ambiguous += other.calls_ambiguous;
        for (k, v) in &other.hub_fanin {
            *self.hub_fanin.entry(*k).or_insert(0) += v;
        }
    }

    pub fn symbols_total(&self) -> u64 {
        self.symbols_by_kind.values().sum()
    }

    pub fn calls_total(&self) -> u64 {
        self.calls_di + self.calls_shared + self.calls_cross_module + self.calls_ambiguous
    }

    /// Planted references plus one structural edge per declared symbol and one per import.
    pub fn edges_estimated(&self) -> u64 {
        self.calls_total() + self.imports + self.symbols_total()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub seed: u64,
    pub profile: String,
    pub generator_version: u32,
    pub counts: Counts,
    pub symbols_total: u64,
    pub edges_estimated: u64,
    /// Fan-in of the ten heaviest hubs, descending.
    pub hub_fanin_top: Vec<u64>,
    /// `blake3` over the sorted `(path, blake3(bytes))` list.
    pub tree_hash: String,
}

impl Manifest {
    pub fn new(seed: u64, profile: &str, counts: Counts, tree_hash: String) -> Self {
        let mut fanin: Vec<u64> = counts.hub_fanin.values().copied().collect();
        fanin.sort_unstable_by(|a, b| b.cmp(a));
        fanin.truncate(10);
        Self {
            seed,
            profile: profile.to_owned(),
            generator_version: GENERATOR_VERSION,
            symbols_total: counts.symbols_total(),
            edges_estimated: counts.edges_estimated(),
            hub_fanin_top: fanin,
            counts,
            tree_hash,
        }
    }

    pub fn same_key(&self, seed: u64, profile: &str) -> bool {
        self.seed == seed && self.profile == profile && self.generator_version == GENERATOR_VERSION
    }
}

/// Order-independent hash of a tree: callers pass `(path, blake3 hex)` pairs in any order.
pub fn tree_hash(mut entries: Vec<(String, String)>) -> String {
    entries.sort();
    let mut hasher = blake3::Hasher::new();
    for (path, hash) in &entries {
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_hash_ignores_input_order() {
        let a = vec![
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "2".to_owned()),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(tree_hash(a), tree_hash(b));
    }

    #[test]
    fn merge_sums_counts() {
        let mut a = Counts::default();
        a.sym("class", 2);
        let mut b = Counts::default();
        b.sym("class", 3);
        b.sym("method", 1);
        a.merge(&b);
        assert_eq!(a.symbols_by_kind["class"], 5);
        assert_eq!(a.symbols_total(), 6);
    }
}
