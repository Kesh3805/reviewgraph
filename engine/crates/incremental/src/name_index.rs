//! Name index with delta support (INC-004).
//!
//! A [`NameDelta`] records which candidate keys were added to or removed from each name by a
//! head commit. [`OverlayNameIndex`] merges it over the base [`NameLookup`], materialising slices
//! only for affected names, so every other lookup is a zero-allocation delegation to the base.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use codegraph::{NameLookup, NodeKey};

/// Names whose candidate sets changed, split by index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameDelta {
    pub added: BTreeMap<String, BTreeSet<NodeKey>>,
    pub removed: BTreeMap<String, BTreeSet<NodeKey>>,
    pub member_added: BTreeMap<String, BTreeSet<NodeKey>>,
    pub member_removed: BTreeMap<String, BTreeSet<NodeKey>>,
}

/// Names a re-link must revisit, plus those whose resolution tier may have flipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AffectedNames {
    pub top: BTreeSet<String>,
    pub members: BTreeSet<String>,
    /// Names whose candidate count crossed 1<->2 or the ambiguity fan-out limit.
    pub threshold_crossed: BTreeSet<String>,
}

type KeyMap = BTreeMap<String, BTreeSet<NodeKey>>;

impl NameDelta {
    /// Records a top-level declaration added under `name`.
    pub fn add_top(&mut self, name: &str, key: NodeKey) {
        self.added.entry(name.to_owned()).or_default().insert(key);
    }

    /// Records a top-level declaration removed from `name`.
    pub fn remove_top(&mut self, name: &str, key: NodeKey) {
        self.removed.entry(name.to_owned()).or_default().insert(key);
    }

    /// Records a member declaration added under `name`.
    pub fn add_member(&mut self, name: &str, key: NodeKey) {
        self.member_added
            .entry(name.to_owned())
            .or_default()
            .insert(key);
    }

    /// Records a member declaration removed from `name`.
    pub fn remove_member(&mut self, name: &str, key: NodeKey) {
        self.member_removed
            .entry(name.to_owned())
            .or_default()
            .insert(key);
    }

    /// True when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.member_added.is_empty()
            && self.member_removed.is_empty()
    }

    /// Names whose merged candidate set differs from the base set, and threshold crossings.
    pub fn affected_names(&self, base: &dyn NameLookup, max_ambiguous_fanout: u8) -> AffectedNames {
        let mut out = AffectedNames::default();
        let fanout = usize::from(max_ambiguous_fanout);
        for name in names(&self.added, &self.removed) {
            let before = base.top(&name);
            let after = merge(before, self.added.get(&name), self.removed.get(&name));
            if before != after.as_slice() {
                if crossed(before.len(), after.len(), fanout) {
                    out.threshold_crossed.insert(name.clone());
                }
                out.top.insert(name);
            }
        }
        for name in names(&self.member_added, &self.member_removed) {
            let before = base.members(&name);
            let after = merge(
                before,
                self.member_added.get(&name),
                self.member_removed.get(&name),
            );
            if before != after.as_slice() {
                if crossed(before.len(), after.len(), fanout) {
                    out.threshold_crossed.insert(name.clone());
                }
                out.members.insert(name);
            }
        }
        out
    }
}

fn names(a: &KeyMap, r: &KeyMap) -> BTreeSet<String> {
    a.keys().chain(r.keys()).cloned().collect()
}

fn crossed(before: usize, after: usize, fanout: usize) -> bool {
    (before <= 1 && after >= 2)
        || (before >= 2 && after <= 1)
        || (before <= fanout) != (after <= fanout)
}

/// `(base ∪ added) \ removed`, sorted and deduplicated.
fn merge(
    base: &[NodeKey],
    added: Option<&BTreeSet<NodeKey>>,
    removed: Option<&BTreeSet<NodeKey>>,
) -> Vec<NodeKey> {
    let mut set: BTreeSet<NodeKey> = base.iter().copied().collect();
    if let Some(a) = added {
        set.extend(a.iter().copied());
    }
    if let Some(r) = removed {
        for key in r {
            set.remove(key);
        }
    }
    set.into_iter().collect()
}

/// Base index plus a delta; unaffected names delegate to the base slice.
pub struct OverlayNameIndex<'a> {
    base: &'a dyn NameLookup,
    merged_top: HashMap<String, Box<[NodeKey]>>,
    merged_members: HashMap<String, Box<[NodeKey]>>,
}

impl std::fmt::Debug for OverlayNameIndex<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayNameIndex")
            .field("merged_top", &self.merged_top.len())
            .field("merged_members", &self.merged_members.len())
            .finish_non_exhaustive()
    }
}

impl<'a> OverlayNameIndex<'a> {
    /// Precomputes merged slices for the names `delta` touches. Cost `O(|delta| log |delta|)`.
    pub fn new(base: &'a dyn NameLookup, delta: &NameDelta) -> Self {
        let mut merged_top = HashMap::new();
        for name in names(&delta.added, &delta.removed) {
            let merged = merge(
                base.top(&name),
                delta.added.get(&name),
                delta.removed.get(&name),
            );
            merged_top.insert(name, merged.into_boxed_slice());
        }
        let mut merged_members = HashMap::new();
        for name in names(&delta.member_added, &delta.member_removed) {
            let merged = merge(
                base.members(&name),
                delta.member_added.get(&name),
                delta.member_removed.get(&name),
            );
            merged_members.insert(name, merged.into_boxed_slice());
        }
        Self {
            base,
            merged_top,
            merged_members,
        }
    }
}

impl NameLookup for OverlayNameIndex<'_> {
    fn top(&self, name: &str) -> &[NodeKey] {
        self.merged_top
            .get(name)
            .map_or_else(|| self.base.top(name), |merged| &merged[..])
    }

    fn members(&self, name: &str) -> &[NodeKey] {
        self.merged_members
            .get(name)
            .map_or_else(|| self.base.members(name), |merged| &merged[..])
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use codegraph::{node_id::NodeId, NameIndex};

    fn key(s: &str) -> NodeKey {
        NodeId::from_canonical(s).key()
    }

    fn base_with(entries: &[(&str, NodeKey)]) -> NameIndex {
        let mut index = NameIndex::default();
        for (name, k) in entries {
            index.insert_top(name, *k);
        }
        index
    }

    #[test]
    fn unique_to_ambiguous_flagged_threshold_crossed() {
        let base = base_with(&[("f", key("ts:a#f/function"))]);
        let mut delta = NameDelta::default();
        delta.add_top("f", key("ts:b#f/function"));
        let affected = delta.affected_names(&base, 3);
        assert!(affected.top.contains("f"));
        assert!(affected.threshold_crossed.contains("f"));
    }

    #[test]
    fn removal_of_only_candidate_makes_name_empty() {
        let k = key("ts:a#f/function");
        let base = base_with(&[("f", k)]);
        let mut delta = NameDelta::default();
        delta.remove_top("f", k);
        let overlay = OverlayNameIndex::new(&base, &delta);
        assert!(overlay.top("f").is_empty());
    }

    #[test]
    fn rename_affects_old_and_new_names() {
        let k = key("ts:a#old/function");
        let base = base_with(&[("old", k)]);
        let mut delta = NameDelta::default();
        delta.remove_top("old", k);
        delta.add_top("new", key("ts:a#new/function"));
        let affected = delta.affected_names(&base, 3);
        assert!(affected.top.contains("old") && affected.top.contains("new"));
    }

    #[test]
    fn unaffected_name_returns_base_slice_pointer_equal() {
        let base = base_with(&[("keep", key("ts:a#keep/function"))]);
        let mut delta = NameDelta::default();
        delta.add_top("other", key("ts:b#other/function"));
        let overlay = OverlayNameIndex::new(&base, &delta);
        assert!(std::ptr::eq(overlay.top("keep"), base.top("keep")));
    }

    #[test]
    fn member_index_delta_tracked_separately() {
        let base = NameIndex::default();
        let mut delta = NameDelta::default();
        delta.add_member("m", key("ts:a#C.m/method"));
        let overlay = OverlayNameIndex::new(&base, &delta);
        assert_eq!(overlay.members("m").len(), 1);
        assert!(overlay.top("m").is_empty());
        let affected = delta.affected_names(&base, 3);
        assert!(affected.members.contains("m") && affected.top.is_empty());
    }
}
