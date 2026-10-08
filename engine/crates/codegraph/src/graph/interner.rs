//! String interning: one copy of every distinct string, addressed by a `u32`.
//!
//! A 1M-node graph repeats qualified names, module paths and type names constantly; keeping
//! one `String` per distinct value and handing out `StrId`s turns every "compare this name"
//! into a `u32` compare and cuts the resident set by an order of magnitude.

use std::collections::HashMap;
use std::fmt;

/// A stable handle to a string interned in an [`Interner`].
///
/// Ids are assigned in insertion order, so an interner that is fed the same strings in the
/// same order yields the same ids. `GraphBuilder` relies on that: it interns in the
/// deterministic order produced by its sorted node/edge/file lists, which is what makes
/// two graphs built from the same multiset of inputs byte-identical.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct StrId(u32);

impl StrId {
    /// The id of the empty string once it has been interned.
    pub const EMPTY: Self = Self(0);

    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    /// `u32::MAX` is reserved as "no such string".
    pub const fn is_none(self) -> bool {
        self.0 == u32::MAX
    }
}

impl fmt::Debug for StrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StrId({})", self.0)
    }
}

impl fmt::Display for StrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for StrId {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<StrId> for u32 {
    fn from(id: StrId) -> u32 {
        id.0
    }
}

/// The interned string table of a [`crate::graph::Graph`].
#[derive(Debug, Clone, Default)]
pub struct Interner {
    strings: Vec<String>,
    index: HashMap<String, StrId>,
}

impl Interner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the id of `s`, interning it first when it is new.
    ///
    /// Returns `None` once the table holds `u32::MAX` distinct strings, which needs more
    /// than 60 GB of RAM and is therefore a capacity answer rather than a reachable one.
    pub fn intern(&mut self, s: &str) -> Option<StrId> {
        if let Some(id) = self.index.get(s) {
            return Some(*id);
        }
        let raw = u32::try_from(self.strings.len()).ok()?;
        self.strings.push(s.to_owned());
        self.index.insert(s.to_owned(), StrId(raw));
        Some(StrId(raw))
    }

    /// The id of `s` when it is already interned.
    pub fn lookup(&self, s: &str) -> Option<StrId> {
        self.index.get(s).copied()
    }

    /// The string behind `id`, or `None` when the id was never interned.
    pub fn get(&self, id: StrId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(String::as_str)
    }

    /// The string behind `id`; only the empty string for an unknown id.
    ///
    /// Prefer [`Self::get`] when a missing id is worth distinguishing.
    pub fn resolve(&self, id: StrId) -> &str {
        self.get(id).unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (StrId, &str)> {
        self.strings
            .iter()
            .enumerate()
            .map(|(i, s)| (StrId(i as u32), s.as_str()))
    }

    /// Bytes held on the heap: the string bytes themselves plus both tables' capacity.
    pub fn heap_size_bytes(&self) -> usize {
        let bytes: usize = self.strings.iter().map(String::len).sum();
        let vec_cap = self.strings.capacity() * size_of::<String>();
        // `HashMap<String, StrId>` stores the key bytes a second time; estimate the bucket
        // array at 16 bytes per entry (8-byte hash, `String`, `StrId`, alignment) plus the
        // 24-byte `String` header.
        let map = self.index.capacity() * (16 + size_of::<String>() + size_of::<StrId>());
        bytes + vec_cap + map
    }
}

impl<S: AsRef<str>> FromIterator<S> for Interner {
    fn from_iter<T: IntoIterator<Item = S>>(iter: T) -> Self {
        let mut interner = Self::new();
        for s in iter {
            let _ = interner.intern(s.as_ref());
        }
        interner
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn interned_strings_get_stable_ids_in_insertion_order() {
        let mut i = Interner::new();
        assert_eq!(i.intern("a"), Some(StrId(0)));
        assert_eq!(i.intern("b"), Some(StrId(1)));
        assert_eq!(i.intern("a"), Some(StrId(0)));
        assert_eq!(i.len(), 2);
        assert_eq!(i.get(StrId(1)), Some("b"));
        assert_eq!(i.lookup("b"), Some(StrId(1)));
        assert_eq!(i.lookup("zzz"), None);
    }

    #[test]
    fn resolve_defaults_to_the_empty_string_for_unknown_ids() {
        let i = Interner::new();
        assert_eq!(i.resolve(StrId(7)), "");
        assert!(i.is_empty());
    }

    #[test]
    fn heap_size_accounts_for_bytes_and_tables() {
        let mut i = Interner::new();
        i.intern("hello");
        i.intern("world");
        let heap = i.heap_size_bytes();
        assert!(heap >= 10, "heap {heap} misses the string bytes");
        assert!(heap < 4096, "heap {heap} over-estimates a 2-string table");
    }

    #[test]
    fn from_iterator_interns_in_order() {
        let i: Interner = ["x", "y", "x"].into_iter().collect();
        assert_eq!(i.len(), 2);
        assert_eq!(i.lookup("y"), Some(StrId(1)));
    }
}
