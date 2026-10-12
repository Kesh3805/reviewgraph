//! Driving embedding sync from incremental invalidations (SEM-008).
//!
//! `semantic` may not depend on `incremental`, so the invalidation set arrives as the small
//! [`InvalidationInput`] defined here; the indexing stage maps INC-008's invalidation set into it
//! (changed/added/removed/renamed symbols, 1-hop dependents with their edge kind, changed docs,
//! profile version change) and supplies unit inputs through [`UnitSource`].

use std::collections::BTreeSet;

use review_core::ids::{RepositoryId, SnapshotId, SymbolKey};
use review_core::location::RepoPath;
use serde::{Deserialize, Serialize};

use crate::collections::CollectionRegistry;
use crate::error::Result;
use crate::index::{DeleteSelector, SemanticIndex};
use crate::metrics;
use crate::sync::{sync, Rekey, SyncOptions, SyncReport, SyncRequest, UnitRef};
use crate::tenant::TenantScope;
use crate::units::{
    code_chunks, convention_unit, doc_units, symbol_summary, ConventionInput, DocInput,
    EmbeddingUnit, SymbolInput, UnitKind, UNIT_TEMPLATE_VERSION,
};

/// Graph edge through which a dependent was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DependentEdge {
    Calls,
    Implements,
    Extends,
    /// Any other edge: does not change summary text, ignored.
    Other,
}

/// A rename or move with lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedSymbol {
    pub from: SymbolKey,
    pub to: SymbolKey,
    /// `body_hash` unchanged: the stored vector can be moved without embedding.
    pub body_unchanged: bool,
}

/// What changed in one delta snapshot, as the semantic layer needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvalidationInput {
    pub changed_symbols: Vec<SymbolKey>,
    pub added_symbols: Vec<SymbolKey>,
    pub removed_symbols: Vec<SymbolKey>,
    pub renamed: Vec<RenamedSymbol>,
    /// 1-hop dependents of changed symbols.
    pub dependents: Vec<(SymbolKey, DependentEdge)>,
    pub changed_docs: Vec<RepoPath>,
    pub removed_docs: Vec<RepoPath>,
    pub profile_version_changed: bool,
}

/// Runtime identity compared against the registry to detect a full re-sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedVersions {
    pub space_id: String,
    pub template_version: u16,
}

impl SyncedVersions {
    pub fn current(space_id: impl Into<String>) -> Self {
        Self {
            space_id: space_id.into(),
            template_version: UNIT_TEMPLATE_VERSION,
        }
    }
}

/// Units to rebuild and remove.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildPlan {
    /// Rebuild everything (space or template version changed).
    pub full: bool,
    /// Summary and code chunks.
    pub symbols_full: BTreeSet<SymbolKey>,
    /// Summary only (dependents whose "calls / called by" lines may change).
    pub symbols_summary_only: BTreeSet<SymbolKey>,
    pub remove_symbols: BTreeSet<SymbolKey>,
    pub renamed: Vec<RenamedSymbol>,
    pub docs: BTreeSet<RepoPath>,
    pub remove_docs: BTreeSet<RepoPath>,
    pub conventions: bool,
}

/// Maps an invalidation set to a rebuild plan. `previous` is what the registry recorded for
/// the last sync; `None` or a mismatch means a full re-sync.
pub fn plan(
    inv: &InvalidationInput,
    previous: Option<&SyncedVersions>,
    runtime: &SyncedVersions,
) -> RebuildPlan {
    if previous != Some(runtime) {
        return RebuildPlan {
            full: true,
            conventions: true,
            ..RebuildPlan::default()
        };
    }
    let mut p = RebuildPlan::default();
    p.symbols_full.extend(
        inv.changed_symbols
            .iter()
            .chain(&inv.added_symbols)
            .copied(),
    );
    for r in &inv.renamed {
        if !r.body_unchanged {
            p.symbols_full.insert(r.to);
        }
    }
    p.renamed = inv
        .renamed
        .iter()
        .filter(|r| r.body_unchanged)
        .cloned()
        .collect();
    p.remove_symbols.extend(inv.removed_symbols.iter().copied());
    for (k, edge) in &inv.dependents {
        if matches!(
            edge,
            DependentEdge::Calls | DependentEdge::Implements | DependentEdge::Extends
        ) && !p.symbols_full.contains(k)
            && !p.remove_symbols.contains(k)
        {
            p.symbols_summary_only.insert(*k);
        }
    }
    p.docs.extend(inv.changed_docs.iter().cloned());
    p.remove_docs.extend(inv.removed_docs.iter().cloned());
    p.conventions = inv.profile_version_changed;
    p
}

/// Unit inputs for the snapshot being synced, provided by the indexing stage.
pub trait UnitSource {
    /// Inputs of the given symbols (unknown keys are skipped).
    fn symbols(&self, keys: &BTreeSet<SymbolKey>) -> Vec<SymbolInput>;
    /// Every symbol of the snapshot (full re-sync).
    fn all_symbols(&self) -> Vec<SymbolInput>;
    fn docs(&self, paths: &BTreeSet<RepoPath>) -> Vec<DocInput>;
    fn all_docs(&self) -> Vec<DocInput>;
    fn conventions(&self) -> Vec<ConventionInput>;
}

/// `semantic-sync` job payload (ADR-012: ids only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticSyncJob {
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    /// Reference to the stored invalidation set (not the set itself).
    pub invalidation_ref: String,
}

impl SemanticSyncJob {
    /// Queue idempotency key `semsync:{repo}:{snapshot}:{space}`.
    pub fn idempotency_key(&self, space_id: &str) -> String {
        format!(
            "semsync:{}:{}:{space_id}",
            self.repository_id, self.snapshot_id
        )
    }
}

/// Builds the units a plan asks for.
pub fn build_units(
    plan: &RebuildPlan,
    source: &dyn UnitSource,
    repo: RepositoryId,
    snapshot: SnapshotId,
) -> Vec<EmbeddingUnit> {
    let mut units = Vec::new();
    let (full, summary_only) = if plan.full {
        (source.all_symbols(), Vec::new())
    } else {
        (
            source.symbols(&plan.symbols_full),
            source.symbols(&plan.symbols_summary_only),
        )
    };
    for s in &full {
        units.extend(symbol_summary(s, repo, snapshot));
        units.extend(code_chunks(s, repo, snapshot));
    }
    for s in &summary_only {
        units.extend(symbol_summary(s, repo, snapshot));
    }
    let docs = if plan.full {
        source.all_docs()
    } else {
        source.docs(&plan.docs)
    };
    for d in &docs {
        units.extend(doc_units(d, repo, snapshot));
    }
    if plan.conventions {
        for c in source.conventions() {
            units.push(convention_unit(&c, repo, snapshot));
        }
    }
    for k in UnitKind::ALL {
        let n = units.iter().filter(|u| u.kind == k).count() as u64;
        if n > 0 {
            metrics::invalidated(k.as_str(), n);
        }
    }
    units
}

#[allow(clippy::too_many_arguments)]
/// Runs a plan: deletes removed symbols and docs (all kinds, scoped), moves unchanged renames,
/// and syncs rebuilt units (content hashes decide what is re-embedded).
pub async fn apply(
    index: &SemanticIndex,
    scope: &TenantScope,
    repo: RepositoryId,
    snapshot: SnapshotId,
    plan: &RebuildPlan,
    source: &dyn UnitSource,
    opts: &SyncOptions,
    registry: Option<&dyn CollectionRegistry>,
) -> Result<SyncReport> {
    let Some(repo_scope) = scope.narrowed(repo) else {
        return Err(crate::Error::ScopeViolation(format!(
            "repository {repo} is not in the scope"
        )));
    };
    if !plan.remove_symbols.is_empty() {
        index
            .delete_units(
                &repo_scope,
                DeleteSelector::SymbolKeys(plan.remove_symbols.iter().copied().collect()),
            )
            .await?;
    }
    if !plan.remove_docs.is_empty() {
        index
            .delete_units(
                &repo_scope,
                DeleteSelector::FilePaths(plan.remove_docs.iter().cloned().collect()),
            )
            .await?;
    }
    let units = build_units(plan, source, repo, snapshot);
    // Unchanged renames: the summary of the new symbol moves from the old key.
    let renamed_targets: BTreeSet<SymbolKey> = plan.renamed.iter().map(|r| r.to).collect();
    let renamed_inputs = source.symbols(&renamed_targets);
    let mut renames = Vec::new();
    for r in &plan.renamed {
        let Some(input) = renamed_inputs.iter().find(|s| s.symbol_key() == r.to) else {
            continue;
        };
        if let Some(to) = symbol_summary(input, repo, snapshot) {
            renames.push(Rekey {
                from: UnitRef {
                    kind: UnitKind::SymbolSummary,
                    key: r.from.to_string(),
                    repository_id: repo,
                },
                to,
            });
        }
        // Chunk keys derive from the symbol key, so chunks are rebuilt; their content hashes
        // are new keys, which costs embeddings only for the moved function's body.
        let chunks = code_chunks(input, repo, snapshot);
        renames.extend(chunks.into_iter().enumerate().map(|(i, to)| Rekey {
            from: UnitRef {
                kind: UnitKind::CodeChunk,
                key: crate::units::chunk_key(&r.from, i),
                repository_id: repo,
            },
            to,
        }));
    }
    sync(
        index,
        &repo_scope,
        SyncRequest {
            snapshot_id: snapshot,
            changed: &units,
            removed: &[],
            renamed: &renames,
        },
        opts,
        registry,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> SymbolKey {
        SymbolKey::from_bytes([n; 16])
    }

    #[test]
    fn version_change_means_full_resync() {
        let now = SyncedVersions::current("hash-fh768-768");
        let old = SyncedVersions {
            template_version: 0,
            ..now.clone()
        };
        assert!(plan(&InvalidationInput::default(), Some(&old), &now).full);
        assert!(plan(&InvalidationInput::default(), None, &now).full);
        assert!(!plan(&InvalidationInput::default(), Some(&now), &now).full);
    }

    #[test]
    fn dependents_get_summary_only_and_other_edges_ignored() {
        let now = SyncedVersions::current("s");
        let inv = InvalidationInput {
            changed_symbols: vec![key(1)],
            dependents: vec![
                (key(2), DependentEdge::Calls),
                (key(3), DependentEdge::Other),
                (key(1), DependentEdge::Calls),
            ],
            ..InvalidationInput::default()
        };
        let p = plan(&inv, Some(&now), &now);
        assert_eq!(p.symbols_full, BTreeSet::from([key(1)]));
        assert_eq!(p.symbols_summary_only, BTreeSet::from([key(2)]));
    }
}
