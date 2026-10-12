//! Context cache keyed by `(review_run, cluster, reviewer, input_hash)` (CTX-009).

use std::collections::BTreeMap;
use std::sync::Mutex;

use review_core::reviewer_type::ReviewerType;

use crate::model::ContextPackage;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CtxKey {
    pub review_run_id: String,
    pub cluster_id: String,
    pub reviewer: ReviewerType,
    pub input_hash: String,
}

/// Cache failures never fail the stage: `get` returns `None`, `put` is best effort.
pub trait ContextCache: Send + Sync {
    fn get(&self, key: &CtxKey) -> Option<ContextPackage>;
    fn put(&self, key: &CtxKey, package: &ContextPackage);
}

/// In-memory cache. First write wins (insert-if-absent), matching `ON CONFLICT DO NOTHING`.
#[derive(Debug, Default)]
pub struct InMemoryContextCache {
    entries: Mutex<BTreeMap<CtxKey, ContextPackage>>,
}

impl InMemoryContextCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ContextCache for InMemoryContextCache {
    fn get(&self, key: &CtxKey) -> Option<ContextPackage> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    fn put(&self, key: &CtxKey, package: &ContextPackage) {
        if let Ok(mut entries) = self.entries.lock() {
            entries
                .entry(key.clone())
                .or_insert_with(|| package.clone());
        }
    }
}
