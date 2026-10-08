//! The repository-wide name index the linker's last cascade steps consult (CG-005).
//!
//! [`NameIndex`] maps a top-level or member name to the sorted keys that declare it. It is the
//! only way the linker turns a bare identifier into an edge, and it is deliberately an
//! *interface* too ([`NameLookup`]): the incremental lane overlays a delta on the base index
//! (INC-004) and a single-file re-link must see exactly what the full build would have seen.

use std::collections::HashMap;

use crate::linker::symbol_table::{FileSymbols, SymbolTable};
use crate::node_id::NodeKey;

/// Read-only name lookup, as the linker consumes it.
///
/// `&[NodeKey]` rather than an iterator so the two implementations can return a slice without
/// allocating on a hot path.
pub trait NameLookup: Send + Sync {
    /// Keys of symbols declared at file scope under `name`, sorted.
    fn top(&self, name: &str) -> &[NodeKey];
    /// Keys of class members named `name` anywhere in the snapshot, sorted.
    fn members(&self, name: &str) -> &[NodeKey];
}

/// The concrete index: name → sorted candidate keys.
///
/// Values are sorted by key bytes so candidate order — and therefore fan-out order — does not
/// depend on how the index was filled.
#[derive(Debug, Clone, Default)]
pub struct NameIndex {
    top: HashMap<String, Vec<NodeKey>>,
    members: HashMap<String, Vec<NodeKey>>,
}

impl NameIndex {
    /// Builds the index from a [`SymbolTable`] in `O(S)`.
    #[must_use]
    pub fn build(table: &SymbolTable) -> Self {
        let mut index = Self::default();
        for file in table.iter().map(|(_, file)| file) {
            index.insert_file(file);
        }
        index.sort();
        index
    }

    fn insert_file(&mut self, file: &FileSymbols) {
        for (raw, symbol) in file.symbols.iter().enumerate() {
            if raw == 0 {
                continue;
            }
            if symbol.parent.is_none() {
                self.top
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(symbol.key);
            }
            if let Some(parent) = symbol.parent {
                self.members
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(symbol.key);
                let _ = parent;
            }
        }
        for (class, table) in &file.members {
            let _ = class;
            for (name, key) in table {
                self.members.entry(name.clone()).or_default().push(*key);
            }
        }
    }

    /// Sorts and de-duplicates every candidate list, making the index order-independent.
    fn sort(&mut self) {
        for values in self.top.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        for values in self.members.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
    }

    /// Adds one candidate to the top-level list of `name`.
    pub fn insert_top(&mut self, name: &str, key: NodeKey) {
        self.top.entry(name.to_owned()).or_default().push(key);
    }

    /// Adds one candidate to the member list of `name`.
    pub fn insert_member(&mut self, name: &str, key: NodeKey) {
        self.members.entry(name.to_owned()).or_default().push(key);
    }

    pub fn top_names(&self) -> usize {
        self.top.len()
    }

    pub fn member_names(&self) -> usize {
        self.members.len()
    }
}

impl NameLookup for NameIndex {
    fn top(&self, name: &str) -> &[NodeKey] {
        self.top.get(name).map_or(&[], Vec::as_slice)
    }

    fn members(&self, name: &str) -> &[NodeKey] {
        self.members.get(name).map_or(&[], Vec::as_slice)
    }
}

/// An overlay index: the base index plus a delta's insertions, used by the incremental lane to
/// re-link an unchanged file against a snapshot that has new declarations (INC-004/006).
///
/// Kept in this crate so the resolution cascade itself needs no knowledge of deltas.
#[derive(Debug, Clone, Default)]
pub struct NameIndexOverlay {
    base: NameIndex,
    added_top: HashMap<String, Vec<NodeKey>>,
    added_members: HashMap<String, Vec<NodeKey>>,
    removed: Vec<NodeKey>,
}

impl NameIndexOverlay {
    #[must_use]
    pub fn new(base: NameIndex) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }

    pub fn add_top(&mut self, name: &str, key: NodeKey) {
        self.added_top.entry(name.to_owned()).or_default().push(key);
    }

    pub fn add_member(&mut self, name: &str, key: NodeKey) {
        self.added_members
            .entry(name.to_owned())
            .or_default()
            .push(key);
    }

    pub fn remove(&mut self, key: NodeKey) {
        self.removed.push(key);
    }
}

impl NameLookup for NameIndexOverlay {
    fn top(&self, name: &str) -> &[NodeKey] {
        match self.added_top.get(name) {
            Some(added) if !added.is_empty() => added,
            _ => self.base.top(name),
        }
    }

    fn members(&self, name: &str) -> &[NodeKey] {
        match self.added_members.get(name) {
            Some(added) if !added.is_empty() => added,
            _ => self.base.members(name),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::linker::symbol_table::TypeRef;
    use crate::node_id::NodeId;

    #[test]
    fn type_ref_strips_what_a_name_lookup_cannot_use() {
        assert_eq!(TypeRef::new("AuthService").base_name(), "AuthService");
        assert_eq!(TypeRef::new("users.AuthService").base_name(), "AuthService");
        assert_eq!(
            TypeRef::new("users.AuthService").member_name(),
            "AuthService"
        );
        assert_eq!(TypeRef::new("Map<string, User>").base_name(), "Map");
        assert_eq!(TypeRef::new("User?").base_name(), "User");
        assert_eq!(TypeRef::new("readonly User").base_name(), "User");
        assert_eq!(TypeRef::new("Promise<User>").base_name(), "Promise");
        assert_eq!(TypeRef::new("User").as_str(), "User");
        assert_eq!(TypeRef::new("User").to_string(), "User");
    }

    #[test]
    fn empty_index_answers_with_empty_slices() {
        let index = NameIndex::default();
        assert!(index.top("anything").is_empty());
        assert!(index.members("anything").is_empty());
        assert_eq!(index.top_names(), 0);
        assert_eq!(index.member_names(), 0);
    }

    #[test]
    fn candidates_are_sorted_and_deduplicated() {
        let mut index = NameIndex::default();
        let a = NodeId::from_canonical("ts:a#A/function").key();
        let b = NodeId::from_canonical("ts:b#B/function").key();
        index.insert_top("shared", b);
        index.insert_top("shared", a);
        index.insert_top("shared", a);
        index.sort();
        let mut expected = vec![b, a];
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(index.top("shared"), expected.as_slice());
        assert_eq!(index.top_names(), 1);
    }

    #[test]
    fn overlay_prefers_delta_candidates_and_falls_back_to_base() {
        let base = {
            let mut index = NameIndex::default();
            let a = NodeId::from_canonical("ts:a#A/function").key();
            index.insert_top("old", a);
            index.sort();
            index
        };
        let new_key = NodeId::from_canonical("ts:c#C/function").key();
        let mut overlay = NameIndexOverlay::new(base);
        assert_eq!(overlay.top("old").len(), 1);
        assert!(overlay.top("new").is_empty());
        overlay.add_top("new", new_key);
        assert_eq!(overlay.top("new"), &[new_key][..]);
        overlay.add_member("member", new_key);
        assert_eq!(overlay.members("member"), &[new_key][..]);
        overlay.remove(new_key);
        assert_eq!(overlay.removed, vec![new_key]);
    }
}
