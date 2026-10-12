//! The head symbol view: fresh tables for changed files over a lazy base (INC-003).
//!
//! `HeadSymbolTables` never materializes the whole repository. A changed file contributes its head
//! `FileSymbols`; every other file is loaded on first use from a [`BaseSymbolSource`] (backed by the
//! IR cache) and memoized. [`BaseUnitSource`] is the port that yields the base `ParsedUnit` of a
//! path; it is also what the symbol differ needs for the base side of a changed file.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};

use analysis_ir::ParsedUnit;
use codegraph::linker::{file_symbols, FileSymbols};
use codegraph::NodeKey;
use review_core::location::RepoPath;

/// Yields the unit a path had at the base commit.
pub trait BaseUnitSource: Send + Sync {
    /// `None` when the file did not exist (or was not indexed) at the base commit.
    fn unit(&self, path: &RepoPath) -> Option<Arc<ParsedUnit>>;
}

/// Yields the symbol table of an unchanged (base) file.
pub trait BaseSymbolSource: Send + Sync {
    fn file_symbols(&self, path: &RepoPath) -> Option<Arc<FileSymbols>>;
}

/// A [`BaseUnitSource`] over an in-memory map that records which paths were requested.
#[derive(Debug, Default)]
pub struct MapUnitSource {
    units: BTreeMap<RepoPath, Arc<ParsedUnit>>,
    reads: Mutex<Vec<RepoPath>>,
}

impl MapUnitSource {
    /// A source over `units`.
    pub fn new(units: impl IntoIterator<Item = Arc<ParsedUnit>>) -> Self {
        Self {
            units: units
                .into_iter()
                .map(|unit| (unit.file.clone(), unit))
                .collect(),
            reads: Mutex::new(Vec::new()),
        }
    }

    /// Every path requested so far, in request order.
    pub fn reads(&self) -> Vec<RepoPath> {
        self.reads
            .lock()
            .map(|reads| reads.clone())
            .unwrap_or_default()
    }
}

impl BaseUnitSource for MapUnitSource {
    fn unit(&self, path: &RepoPath) -> Option<Arc<ParsedUnit>> {
        if let Ok(mut reads) = self.reads.lock() {
            reads.push(path.clone());
        }
        self.units.get(path).cloned()
    }
}

/// Memoizes `FileSymbols::from_unit` over a [`BaseUnitSource`].
pub struct MemoSymbolSource {
    units: Arc<dyn BaseUnitSource>,
    memo: Mutex<HashMap<RepoPath, Option<Arc<FileSymbols>>>>,
}

impl fmt::Debug for MemoSymbolSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoSymbolSource").finish_non_exhaustive()
    }
}

impl MemoSymbolSource {
    /// Wraps `units`.
    pub fn new(units: Arc<dyn BaseUnitSource>) -> Self {
        Self {
            units,
            memo: Mutex::new(HashMap::new()),
        }
    }
}

impl BaseSymbolSource for MemoSymbolSource {
    fn file_symbols(&self, path: &RepoPath) -> Option<Arc<FileSymbols>> {
        if let Ok(memo) = self.memo.lock() {
            if let Some(found) = memo.get(path) {
                return found.clone();
            }
        }
        let loaded = self
            .units
            .unit(path)
            .map(|unit| Arc::new(file_symbols(&unit)));
        if let Ok(mut memo) = self.memo.lock() {
            memo.insert(path.clone(), loaded.clone());
        }
        loaded
    }
}

/// Fresh `FileSymbols` for changed files over a lazy base view.
pub struct HeadSymbolTables {
    changed: BTreeMap<RepoPath, Arc<FileSymbols>>,
    removed: BTreeSet<RepoPath>,
    key_files: HashMap<NodeKey, RepoPath>,
    base: Arc<dyn BaseSymbolSource>,
}

impl fmt::Debug for HeadSymbolTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeadSymbolTables")
            .field("changed", &self.changed.len())
            .field("removed", &self.removed.len())
            .finish_non_exhaustive()
    }
}

impl HeadSymbolTables {
    /// `changed_units` are the head units of changed files; `removed` are paths that no longer
    /// exist. A path in both counts as changed.
    pub fn new(
        changed_units: &[Arc<ParsedUnit>],
        removed: BTreeSet<RepoPath>,
        base: Arc<dyn BaseSymbolSource>,
    ) -> Self {
        let mut changed = BTreeMap::new();
        let mut key_files = HashMap::new();
        for unit in changed_units {
            let symbols = file_symbols(unit);
            for symbol in symbols.symbols.iter().skip(1) {
                key_files.insert(symbol.key, unit.file.clone());
            }
            key_files.insert(symbols.file_key, unit.file.clone());
            changed.insert(unit.file.clone(), Arc::new(symbols));
        }
        let removed = removed
            .into_iter()
            .filter(|path| !changed.contains_key(path))
            .collect();
        Self {
            changed,
            removed,
            key_files,
            base,
        }
    }

    /// The head symbols of `path`: `None` when removed, fresh when changed, lazily loaded from the
    /// base otherwise.
    pub fn file_symbols(&self, path: &RepoPath) -> Option<Arc<FileSymbols>> {
        if self.removed.contains(path) {
            return None;
        }
        if let Some(fresh) = self.changed.get(path) {
            return Some(Arc::clone(fresh));
        }
        self.base.file_symbols(path)
    }

    /// Head symbols of every changed file, in path order.
    pub fn changed(&self) -> impl Iterator<Item = (&RepoPath, &Arc<FileSymbols>)> {
        self.changed.iter()
    }

    /// Whether `path` was removed at head.
    pub fn is_removed(&self, path: &RepoPath) -> bool {
        self.removed.contains(path)
    }

    /// The changed file that declares `key`, for keys that only exist at head.
    pub fn changed_file_of(&self, key: &NodeKey) -> Option<&RepoPath> {
        self.key_files.get(key)
    }
}
