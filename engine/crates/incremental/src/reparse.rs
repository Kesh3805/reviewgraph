//! Hash-skip and reparse-only-changed (INC-002).
//!
//! For every changed path the head blob is read and hashed. A hash equal to the base file's hash
//! means the path needs no work at all ([`FileOutcome::Unchanged`]); otherwise the unit comes from
//! the unit cache or, on a miss, from the analyzer. Files that are not in the change set are never
//! read, hashed or parsed: this is the property the INC-010 counters assert.
//!
//! The cache and the analyzer are ports ([`UnitCache`], [`UnitParser`]) so the pipeline can back
//! them with the IDX-005 parse cache and the language analyzer registry, and tests can count calls.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use analysis_ir::ParsedUnit;
use codegraph::GraphQuery;
use rayon::prelude::*;
use review_core::location::{ContentHash, RepoPath};

use crate::blob_source::{BlobRead, BlobSource};
use crate::changeset::{ChangeSet, PathChangeKind};
use crate::error::{Error, Result};
use crate::metrics::IncrementalCounters;

/// Cache identity of one parsed file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnitKey {
    pub path: RepoPath,
    pub content_hash: ContentHash,
    pub analyzer_version: String,
}

/// A store of parsed units keyed by `(path, content hash, analyzer version)`.
pub trait UnitCache: Send + Sync {
    /// The cached unit, if any.
    fn lookup(&self, key: &UnitKey) -> Option<Arc<ParsedUnit>>;
    /// Best-effort store.
    fn store(&self, key: UnitKey, unit: Arc<ParsedUnit>);
}

/// Parses file bytes into a unit.
pub trait UnitParser: Send + Sync {
    /// Whether any analyzer handles this path.
    fn supports(&self, path: &RepoPath) -> bool;
    /// The analyzer version part of the cache key for this path.
    fn analyzer_version(&self, path: &RepoPath) -> String;
    /// Parses `bytes`. A parse that merely produced diagnostics is `Ok` with a `Partial` or
    /// `Failed` status; `Err` is reserved for the analyzer itself breaking.
    fn parse(
        &self,
        path: &RepoPath,
        bytes: &[u8],
        content_hash: ContentHash,
    ) -> std::result::Result<ParsedUnit, String>;
}

/// A process-local [`UnitCache`].
#[derive(Debug, Default)]
pub struct InMemoryUnitCache {
    entries: Mutex<HashMap<UnitKey, Arc<ParsedUnit>>>,
}

impl InMemoryUnitCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cached units.
    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl UnitCache for InMemoryUnitCache {
    fn lookup(&self, key: &UnitKey) -> Option<Arc<ParsedUnit>> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    fn store(&self, key: UnitKey, unit: Arc<ParsedUnit>) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(key, unit);
        }
    }
}

/// Why a changed path was not parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Larger than `max_file_bytes`.
    TooLarge,
    /// Binary content.
    Binary,
    /// No analyzer handles the path.
    Unsupported,
}

/// What happened to one changed path.
#[derive(Debug, Clone)]
pub enum FileOutcome {
    /// The head content hash equals the base file's: nothing to do.
    Unchanged {
        path: RepoPath,
        content_hash: ContentHash,
    },
    /// A unit is available for the head content.
    Reparsed {
        path: RepoPath,
        unit: Arc<ParsedUnit>,
        /// The unit came from the cache, not the analyzer.
        cache_hit: bool,
        /// The base file's content hash, when the path existed at the base.
        base_hash: Option<ContentHash>,
        /// Source path when the file was renamed.
        renamed_from: Option<RepoPath>,
    },
    /// The path no longer exists (also emitted for the old path of a rename).
    Deleted { path: RepoPath },
    /// The path changed but cannot be parsed.
    Skipped {
        path: RepoPath,
        reason: SkipReason,
        content_hash: Option<ContentHash>,
    },
}

impl FileOutcome {
    /// The path this outcome is about.
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::Unchanged { path, .. }
            | Self::Reparsed { path, .. }
            | Self::Deleted { path }
            | Self::Skipped { path, .. } => path,
        }
    }
}

/// Counters of one reparse run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReparseCounters {
    /// Changed-path entries examined (not counting the synthetic delete of a rename source).
    pub changed: u32,
    /// Entries skipped because the hash equalled the base hash.
    pub unchanged: u32,
    /// Units produced by the analyzer.
    pub parsed: u32,
    /// Units served from the cache.
    pub cache_hits: u32,
}

/// Outcomes sorted by path plus counters.
#[derive(Debug, Clone)]
pub struct ReparseResult {
    pub outcomes: Vec<FileOutcome>,
    pub counters: ReparseCounters,
}

impl ReparseResult {
    /// The units that must be linked at head, in path order.
    pub fn units(&self) -> Vec<Arc<ParsedUnit>> {
        self.outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                FileOutcome::Reparsed { unit, .. } => Some(Arc::clone(unit)),
                _ => None,
            })
            .collect()
    }

    /// Paths that no longer exist at head.
    pub fn deleted(&self) -> Vec<RepoPath> {
        self.outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                FileOutcome::Deleted { path } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Everything [`reparse_changed`] needs.
pub struct ReparseCx<'a> {
    /// The base graph; only `file(path).content_hash` is consulted.
    pub base: &'a dyn GraphQuery,
    pub blobs: &'a dyn BlobSource,
    pub parser: &'a dyn UnitParser,
    pub cache: Option<&'a dyn UnitCache>,
    pub max_file_bytes: u64,
    pub counters: &'a IncrementalCounters,
}

impl fmt::Debug for ReparseCx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReparseCx")
            .field("max_file_bytes", &self.max_file_bytes)
            .field("cache", &self.cache.is_some())
            .finish_non_exhaustive()
    }
}

/// Reads, hashes and (when the hash differs from the base) parses every changed path.
///
/// Work is proportional to the bytes of the changed files; the repository size never enters. Blob
/// read failure for any path fails the whole update, because a partial head graph is worse than
/// none.
pub fn reparse_changed(cx: &ReparseCx<'_>, changes: &ChangeSet) -> Result<ReparseResult> {
    let span = tracing::info_span!("incremental.reparse", changed = changes.entries.len());
    let _guard = span.enter();

    let slots: Vec<Result<FileOutcome>> = changes
        .entries
        .par_iter()
        .map(|entry| {
            if entry.kind == PathChangeKind::Deleted {
                return Ok(FileOutcome::Deleted {
                    path: entry.path.clone(),
                });
            }
            process(cx, entry)
        })
        .collect();

    let mut outcomes: Vec<FileOutcome> = Vec::with_capacity(slots.len());
    let mut counters = ReparseCounters::default();
    for (entry, slot) in changes.entries.iter().zip(slots) {
        let outcome = slot?;
        if entry.kind != PathChangeKind::Deleted {
            counters.changed += 1;
        }
        match &outcome {
            FileOutcome::Unchanged { .. } => counters.unchanged += 1,
            FileOutcome::Reparsed { cache_hit, .. } => {
                if *cache_hit {
                    counters.cache_hits += 1;
                } else {
                    counters.parsed += 1;
                }
            }
            _ => {}
        }
        if let Some(from) = &entry.renamed_from {
            outcomes.push(FileOutcome::Deleted { path: from.clone() });
        }
        outcomes.push(outcome);
    }
    outcomes.sort_by(|a, b| {
        a.path()
            .as_str()
            .cmp(b.path().as_str())
            .then_with(|| rank(a).cmp(&rank(b)))
    });
    outcomes.dedup_by(|later, earlier| {
        // A rename source that is also an independent deleted entry appears twice.
        matches!(
            (&*later, &*earlier),
            (FileOutcome::Deleted { path: a }, FileOutcome::Deleted { path: b }) if a == b
        )
    });

    cx.counters
        .add_files_skipped_unchanged(u64::from(counters.unchanged));
    cx.counters
        .add_files_reparsed(u64::from(counters.parsed + counters.cache_hits));
    cx.counters
        .add_files_reparsed_analyzer(u64::from(counters.parsed));
    cx.counters
        .add_files_reparsed_cache(u64::from(counters.cache_hits));
    Ok(ReparseResult { outcomes, counters })
}

fn rank(outcome: &FileOutcome) -> u8 {
    match outcome {
        FileOutcome::Deleted { .. } => 0,
        _ => 1,
    }
}

fn process(cx: &ReparseCx<'_>, entry: &crate::changeset::PathChange) -> Result<FileOutcome> {
    let path = &entry.path;
    let read = cx
        .blobs
        .read(path, entry.head_oid.as_deref(), cx.max_file_bytes)
        .map_err(|e| Error::BlobRead {
            path: path.as_str().to_owned(),
            reason: e.0,
        })?;
    let bytes = match read {
        BlobRead::Bytes(bytes) => bytes,
        BlobRead::TooLarge(_) => {
            return Ok(FileOutcome::Skipped {
                path: path.clone(),
                reason: SkipReason::TooLarge,
                content_hash: None,
            })
        }
        BlobRead::Binary => {
            return Ok(FileOutcome::Skipped {
                path: path.clone(),
                reason: SkipReason::Binary,
                content_hash: None,
            })
        }
    };
    let hash = ContentHash::of(&bytes);

    // A renamed file is a new path: `base.file(new)` is `None`, so it is reparsed unless cached.
    let base_hash = cx.base.file(path.as_str()).map(|file| file.content_hash);
    if base_hash == Some(hash) {
        return Ok(FileOutcome::Unchanged {
            path: path.clone(),
            content_hash: hash,
        });
    }
    if !cx.parser.supports(path) {
        return Ok(FileOutcome::Skipped {
            path: path.clone(),
            reason: SkipReason::Unsupported,
            content_hash: Some(hash),
        });
    }

    let key = UnitKey {
        path: path.clone(),
        content_hash: hash,
        analyzer_version: cx.parser.analyzer_version(path),
    };
    if let Some(unit) = cx.cache.and_then(|cache| cache.lookup(&key)) {
        return Ok(FileOutcome::Reparsed {
            path: path.clone(),
            unit,
            cache_hit: true,
            base_hash,
            renamed_from: entry.renamed_from.clone(),
        });
    }
    let unit = cx
        .parser
        .parse(path, &bytes, hash)
        .map_err(|reason| Error::BlobRead {
            path: path.as_str().to_owned(),
            reason,
        })?;
    let unit = Arc::new(unit);
    if let Some(cache) = cx.cache {
        cache.store(key, Arc::clone(&unit));
    }
    Ok(FileOutcome::Reparsed {
        path: path.clone(),
        unit,
        cache_hit: false,
        base_hash,
        renamed_from: entry.renamed_from.clone(),
    })
}
