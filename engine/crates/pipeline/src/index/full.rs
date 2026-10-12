//! The full index pipeline (IDX-001): walk, parse, link, build, validate and persist one
//! snapshot.
//!
//! CPU work (hashing, parsing, linking) runs on a dedicated rayon pool inside `spawn_blocking`;
//! the async side owns persistence, progress and cancellation, which is checked between chunks
//! and phases. File versions are upserted after linking, because the symbol set stored with a
//! file version (including its file node) only exists once the linker has run.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use analysis_ir::{AnalyzerRegistry, ParsedUnit};
use codegraph::{
    validate, EdgeKind, FileInput, Graph as CodeGraph, GraphBuilder, LinkConfig, LinkInput, Linker,
    NodeId, NodeInput, NodeKind, LINKER_VERSION, SCHEMA_VERSION,
};
use graph_storage::convert::{assign_file_versions, from_codegraph, symbols_by_file};
use graph_storage::status::SnapshotStatus;
use graph_storage::types::{
    FileParseStatus, FileVersionInput, NewSnapshot, SnapshotKind, SnapshotMeta, SnapshotPurpose,
    SnapshotQuery,
};
use graph_storage::{GraphStore, StoreError};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, SnapshotId};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use super::config::IndexConfig;
use super::error::IndexError;
use super::parse::{hash_files, parse_files, FileOutcome, ParsedFile};
use super::parse_cache::{CacheLookup, ParseCache, ParseCacheKey};
use super::progress::{IndexPhase, ProgressSink};
use super::resolver::FileSetResolver;
use super::source::{walk_source, SourceFile, SourceSpec, WalkedSource};
use super::tolerance::{FileOutcomeKind, OutcomeCounts, TolerancePolicy};
use super::versions::{compute_versions, ConfigComponents, VersionInputs};

/// How many failing paths `IndexStats::failed_paths_sample` keeps.
pub const FAILED_PATH_SAMPLE: usize = 20;

/// What to index.
#[derive(Debug, Clone)]
pub struct FullIndexRequest {
    pub scope: RepoScope,
    pub commit: CommitSha,
    pub source: SourceSpec,
    pub purpose: SnapshotPurpose,
    pub config: IndexConfig,
    /// Name of the repository node (`repo:/`).
    pub repository_name: String,
    /// Index even when a ready snapshot with the same fingerprint exists.
    pub rebuild: bool,
    pub cancel: CancellationToken,
}

impl FullIndexRequest {
    pub fn new(
        scope: RepoScope,
        commit: CommitSha,
        source: SourceSpec,
        purpose: SnapshotPurpose,
    ) -> Self {
        Self {
            scope,
            commit,
            source,
            purpose,
            config: IndexConfig::default(),
            repository_name: "repository".to_owned(),
            rebuild: false,
            cancel: CancellationToken::new(),
        }
    }
}

/// Counters of one run, recorded in `snapshots.stats` / `index_runs.stats` (paths and counts
/// only, never file content).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IndexStats {
    pub files_walked: u64,
    pub files_ok: u32,
    pub files_partial: u32,
    pub files_failed: u32,
    pub files_skipped: u32,
    /// Files an analyzer actually ran on.
    pub files_parsed: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub nodes: u64,
    pub edges: u64,
    pub unresolved: u64,
    pub validation_warnings: u64,
    pub fact_issues: u64,
    pub diagnostics_written: u64,
    /// Up to [`FAILED_PATH_SAMPLE`] failing paths, sorted.
    pub failed_paths_sample: Vec<String>,
    pub walk_ms: u64,
    pub parse_ms: u64,
    pub link_ms: u64,
    pub build_ms: u64,
    pub persist_ms: u64,
}

impl IndexStats {
    pub fn counts(&self) -> OutcomeCounts {
        OutcomeCounts {
            ok: self.files_ok,
            partial: self.files_partial,
            failed: self.files_failed,
            skipped: self.files_skipped,
        }
    }
}

/// Result of [`FullIndexer::run`].
#[derive(Debug)]
pub struct FullIndexOutcome {
    pub snapshot: SnapshotMeta,
    /// The linked graph; `None` when a ready snapshot with the same fingerprint was reused.
    pub graph: Option<Arc<CodeGraph>>,
    pub stats: IndexStats,
    /// A ready snapshot with the same fingerprint existed and nothing was indexed.
    pub reused: bool,
}

/// Composes walk, analyzers, linker, builder, validator and a [`GraphStore`].
pub struct FullIndexer {
    store: Arc<dyn GraphStore>,
    analyzers: AnalyzerRegistry,
    parser_versions: BTreeMap<String, String>,
    parse_cache: Option<Arc<dyn ParseCache>>,
    diagnostics: Option<Arc<dyn graph_storage::diagnostics::ParseDiagnosticsStore>>,
    pool: Arc<rayon::ThreadPool>,
    tolerance: TolerancePolicy,
    link_config: LinkConfig,
}

impl fmt::Debug for FullIndexer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FullIndexer")
            .field("analyzers", &self.analyzers)
            .field("parse_cache", &self.parse_cache.is_some())
            .field("threads", &self.pool.current_num_threads())
            .field("tolerance", &self.tolerance)
            .finish_non_exhaustive()
    }
}

fn worker(e: impl fmt::Display) -> IndexError {
    IndexError::Worker(e.to_string())
}

impl FullIndexer {
    /// An indexer with a dedicated pool of `threads` workers (at least one).
    pub fn new(
        store: Arc<dyn GraphStore>,
        analyzers: AnalyzerRegistry,
        threads: usize,
    ) -> Result<Self, IndexError> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .thread_name(|i| format!("index-parse-{i}"))
            .build()
            .map_err(worker)?;
        Ok(Self {
            store,
            analyzers,
            parser_versions: BTreeMap::new(),
            parse_cache: None,
            diagnostics: None,
            pool: Arc::new(pool),
            tolerance: TolerancePolicy::default(),
            link_config: LinkConfig::default(),
        })
    }

    /// Parser stack versions recorded in the fingerprint (`tree-sitter`, ...).
    #[must_use]
    pub fn with_parser_versions(mut self, versions: BTreeMap<String, String>) -> Self {
        self.parser_versions = versions;
        self
    }

    #[must_use]
    pub fn with_diagnostics(
        mut self,
        store: Arc<dyn graph_storage::diagnostics::ParseDiagnosticsStore>,
    ) -> Self {
        self.diagnostics = Some(store);
        self
    }

    #[must_use]
    pub fn with_parse_cache(mut self, cache: Arc<dyn ParseCache>) -> Self {
        self.parse_cache = Some(cache);
        self
    }

    #[must_use]
    pub fn with_tolerance(mut self, tolerance: TolerancePolicy) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Indexes `req` into a new full snapshot, or returns the ready one with the same
    /// fingerprint. A fatal error leaves the snapshot `failed`.
    pub async fn run(
        &self,
        req: FullIndexRequest,
        progress: &dyn ProgressSink,
    ) -> Result<FullIndexOutcome, IndexError> {
        let span = tracing::info_span!(
            "repository_index",
            repository_id = ?req.scope.repository_id,
            organization_id = ?req.scope.organization_id,
            commit_sha = %req.commit,
        );
        self.run_inner(req, progress).instrument(span).await
    }

    async fn run_inner(
        &self,
        req: FullIndexRequest,
        progress: &dyn ProgressSink,
    ) -> Result<FullIndexOutcome, IndexError> {
        let mut stats = IndexStats::default();
        if req.cancel.is_cancelled() {
            return Err(IndexError::Cancelled);
        }

        // 1. Walk.
        progress.phase(IndexPhase::Walking).await;
        let started = Instant::now();
        let walked = {
            let source = req.source.clone();
            let cfg = req.config.clone();
            let analyzers = self.analyzers.clone();
            tokio::task::spawn_blocking(move || walk_source(&source, &cfg, &analyzers))
                .await
                .map_err(worker)??
        };
        stats.walk_ms = elapsed_ms(started);
        stats.files_walked = walked.files.len() as u64;

        // 2. Versions and short-circuit.
        let versions = self.versions_for(&req, &walked);
        if !req.rebuild {
            let query =
                SnapshotQuery::by_fingerprint(versions.fingerprint).with_kind(SnapshotKind::Full);
            if let Some(existing) = self.store.find_ready(&req.scope, query).await? {
                return Ok(FullIndexOutcome {
                    snapshot: existing,
                    graph: None,
                    stats,
                    reused: true,
                });
            }
        }

        // 3. Snapshot row.
        let mut new = NewSnapshot::full(
            req.scope,
            req.commit.clone(),
            req.purpose,
            versions.fingerprint,
        );
        new.versions = versions;
        let meta = self.store.create_snapshot(new).await?;
        let id = meta.id;
        progress.started(id).await;
        let moved = self
            .store
            .transition(
                &req.scope,
                id,
                SnapshotStatus::Pending,
                SnapshotStatus::Indexing,
                None,
            )
            .await;
        let outcome = match moved {
            Ok(true) => self.execute(&req, id, walked, stats, progress).await,
            Ok(false) => Err(IndexError::Store(StoreError::Conflict(
                "snapshot left pending by another worker".to_owned(),
            ))),
            Err(e) => Err(IndexError::Store(e)),
        };
        match outcome {
            Ok(done) => {
                progress.phase(IndexPhase::Done).await;
                progress.finish(&done.stats).await;
                Ok(done)
            }
            Err(error) => {
                progress.phase(IndexPhase::Failed).await;
                progress.failed(error.code()).await;
                self.fail(&req.scope, id, &error).await;
                Err(error)
            }
        }
    }

    fn versions_for(
        &self,
        req: &FullIndexRequest,
        walked: &WalkedSource,
    ) -> graph_storage::types::SnapshotVersions {
        let mut analyzer_versions: BTreeMap<String, String> = self
            .analyzers
            .analyzers()
            .iter()
            .map(|a| {
                let id = a.id();
                (id.name, id.version.to_string())
            })
            .collect();
        analyzer_versions.insert("linker".to_owned(), LINKER_VERSION.to_owned());
        let repository_id = req.scope.repository_id.as_uuid().to_string();
        compute_versions(&VersionInputs {
            repository_id: &repository_id,
            commit_sha: req.commit.as_str(),
            analyzer_versions,
            parser_versions: self.parser_versions.clone(),
            config: ConfigComponents::from_canonical(b"", &walked.tsconfig_canonical, b"", b""),
            profile_version: None,
        })
    }

    /// Moves the snapshot to `failed` from whichever active state it is in. Best effort: the
    /// original error is what the caller sees.
    async fn fail(&self, scope: &RepoScope, id: SnapshotId, error: &IndexError) {
        let mut message = format!("{}: {error}", error.code());
        message.truncate(500);
        for from in [SnapshotStatus::Indexing, SnapshotStatus::Persisting] {
            match self
                .store
                .transition(scope, id, from, SnapshotStatus::Failed, Some(&message))
                .await
            {
                Ok(true) => return,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "could not mark snapshot failed");
                    return;
                }
            }
        }
    }

    async fn execute(
        &self,
        req: &FullIndexRequest,
        id: SnapshotId,
        walked: WalkedSource,
        mut stats: IndexStats,
        progress: &dyn ProgressSink,
    ) -> Result<FullIndexOutcome, IndexError> {
        let cfg = &req.config;

        // 4. Parse, chunk by chunk (bounded memory, cancellation points).
        progress.phase(IndexPhase::Parsing).await;
        let started = Instant::now();
        let total = walked.files.len() as u64;
        let mut parsed: Vec<ParsedFile> = Vec::with_capacity(walked.files.len());
        for chunk in walked.files.chunks(cfg.persist_batch.max(1)) {
            if req.cancel.is_cancelled() {
                return Err(IndexError::Cancelled);
            }
            let batch = self.parse_chunk(&walked, chunk, cfg).await?;
            parsed.extend(batch);
            progress.files(parsed.len() as u64, total).await;
        }
        stats.parse_ms = elapsed_ms(started);

        let mut counts = OutcomeCounts::default();
        for p in &parsed {
            counts.record(p.outcome.kind());
            if p.from_cache {
                stats.cache_hits += 1;
            } else if !matches!(p.outcome, FileOutcome::Skipped { .. }) {
                stats.files_parsed += 1;
                if self.parse_cache.is_some() {
                    stats.cache_misses += 1;
                }
            }
        }
        for _ in 0..(walked.skipped_too_large.len() + walked.skipped_binary.len()) {
            counts.record(FileOutcomeKind::Skipped);
        }
        stats.files_ok = counts.ok;
        stats.files_partial = counts.partial;
        stats.files_failed = counts.failed;
        stats.files_skipped = counts.skipped;
        let mut failed: Vec<String> = parsed
            .iter()
            .filter(|p| p.outcome.kind() == FileOutcomeKind::Failed)
            .map(|p| p.file.path.as_str().to_owned())
            .collect();
        failed.sort();
        failed.truncate(FAILED_PATH_SAMPLE);
        stats.failed_paths_sample = failed;
        if let Err(exceeded) = self.tolerance.check(&counts) {
            tracing::warn!(
                failed = exceeded.failed,
                total = exceeded.total,
                "index.tolerance_exceeded"
            );
            return Err(exceeded.into());
        }
        if req.cancel.is_cancelled() {
            return Err(IndexError::Cancelled);
        }

        // 5. Link.
        progress.phase(IndexPhase::Linking).await;
        let started = Instant::now();
        let units: Vec<Arc<ParsedUnit>> = parsed
            .iter()
            .filter_map(|p| p.outcome.unit().cloned())
            .collect();
        let resolver = FileSetResolver::new(walked.all_paths.iter());
        let link_cfg = self.link_config;
        let repository_name = req.repository_name.clone();
        let pool = Arc::clone(&self.pool);
        let linked = tokio::task::spawn_blocking(move || {
            pool.install(|| {
                Linker::link_all(LinkInput {
                    units: &units,
                    resolver: &resolver,
                    config: &link_cfg,
                    repository_name: &repository_name,
                })
            })
        })
        .await
        .map_err(worker)?;
        stats.link_ms = elapsed_ms(started);
        stats.fact_issues = linked.fact_issues.len() as u64;

        // 6. Build and validate.
        progress.phase(IndexPhase::Building).await;
        let started = Instant::now();
        let graph = build_graph(&parsed, linked)?;
        let report = validate(&graph, SCHEMA_VERSION);
        if !report.is_ok() {
            return Err(IndexError::Inconsistent {
                errors: report.errors.len(),
            });
        }
        stats.validation_warnings = report.warnings.len() as u64;
        let graph_stats = graph.stats();
        stats.nodes = graph_stats.nodes as u64;
        stats.edges = graph_stats.edges as u64;
        stats.unresolved = graph_stats.unresolved as u64;
        stats.build_ms = elapsed_ms(started);
        if req.cancel.is_cancelled() {
            return Err(IndexError::Cancelled);
        }

        // 7. Persist.
        progress.phase(IndexPhase::Persisting).await;
        let started = Instant::now();
        let moved = self
            .store
            .transition(
                &req.scope,
                id,
                SnapshotStatus::Indexing,
                SnapshotStatus::Persisting,
                None,
            )
            .await?;
        if !moved {
            return Err(IndexError::Store(StoreError::Conflict(
                "snapshot is no longer indexing".to_owned(),
            )));
        }
        let mut stored = from_codegraph(&graph)?;
        let ids = self
            .upsert_versions(&req.scope, &stored, &parsed, cfg.persist_batch.max(1))
            .await?;
        assign_file_versions(&mut stored, &ids);
        if let Some(diagnostics) = &self.diagnostics {
            let mut rows = Vec::new();
            for p in &parsed {
                if let Some(version) = ids.get(p.file.path.as_str()) {
                    let list = super::diagnostics::rows_for(p);
                    if !list.is_empty() {
                        rows.push((*version, list));
                    }
                }
            }
            stats.diagnostics_written = diagnostics.write(&req.scope, &rows).await?;
        }
        self.store.write_full(&req.scope, id, &stored).await?;
        let ready = self
            .store
            .transition(
                &req.scope,
                id,
                SnapshotStatus::Persisting,
                SnapshotStatus::Ready,
                None,
            )
            .await?;
        if !ready {
            return Err(IndexError::Store(StoreError::Conflict(
                "snapshot is no longer persisting".to_owned(),
            )));
        }
        stats.persist_ms = elapsed_ms(started);
        let snapshot = self
            .store
            .snapshot(&req.scope, id)
            .await?
            .ok_or(IndexError::Store(StoreError::NotFound(id)))?;
        Ok(FullIndexOutcome {
            snapshot,
            graph: Some(Arc::new(graph)),
            stats,
            reused: false,
        })
    }

    /// Hash, look up the cache, parse the misses and store them.
    async fn parse_chunk(
        &self,
        walked: &WalkedSource,
        chunk: &[SourceFile],
        cfg: &IndexConfig,
    ) -> Result<Vec<ParsedFile>, IndexError> {
        let files = chunk.to_vec();
        let cached = self.cached_units(&walked.root, &files, cfg).await?;
        let root = walked.root.clone();
        let analyzers = self.analyzers.clone();
        let cfg = cfg.clone();
        let pool = Arc::clone(&self.pool);
        let parsed = tokio::task::spawn_blocking(move || {
            pool.install(|| parse_files(&root, &files, &analyzers, &cfg, &cached))
        })
        .await
        .map_err(worker)?;
        if let Some(cache) = &self.parse_cache {
            for p in &parsed {
                if p.from_cache {
                    continue;
                }
                if let FileOutcome::Ok(unit) | FileOutcome::Partial(unit) = &p.outcome {
                    // Best effort: a failed store only costs a re-parse next time.
                    if let Err(error) = cache.store(p.cache_key(), Arc::clone(unit)).await {
                        tracing::warn!(%error, "parse cache store failed");
                    }
                }
            }
        }
        Ok(parsed)
    }

    /// Cache hits for `files`, by index; empty when there is no cache or it is unavailable.
    async fn cached_units(
        &self,
        root: &std::path::Path,
        files: &[SourceFile],
        cfg: &IndexConfig,
    ) -> Result<Vec<Option<Arc<ParsedUnit>>>, IndexError> {
        let Some(cache) = &self.parse_cache else {
            return Ok(Vec::new());
        };
        let hashes = {
            let root = root.to_path_buf();
            let files = files.to_vec();
            let cfg = cfg.clone();
            let pool = Arc::clone(&self.pool);
            tokio::task::spawn_blocking(move || pool.install(|| hash_files(&root, &files, &cfg)))
                .await
                .map_err(worker)?
        };
        let mut keys: Vec<ParseCacheKey> = Vec::new();
        let mut positions: Vec<usize> = Vec::new();
        for (i, (file, hash)) in files.iter().zip(hashes).enumerate() {
            let (Some(hash), Some(analyzer)) = (hash, self.analyzers.for_path(&file.path)) else {
                continue;
            };
            keys.push(ParseCacheKey {
                path: file.path.as_str().to_owned(),
                content_hash: *hash.as_bytes(),
                analyzer_version: analyzer.id().version.to_string(),
            });
            positions.push(i);
        }
        let mut out: Vec<Option<Arc<ParsedUnit>>> = vec![None; files.len()];
        match cache.lookup_many(&keys).await {
            Ok(results) => {
                for (position, lookup) in positions.into_iter().zip(results) {
                    if let CacheLookup::Hit(unit) = lookup {
                        out[position] = Some(unit);
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "parse cache unavailable; parsing everything"),
        }
        Ok(out)
    }

    /// Upserts one file version per file of the graph, in batches; returns `path -> id`.
    async fn upsert_versions(
        &self,
        scope: &RepoScope,
        graph: &graph_storage::model::Graph,
        parsed: &[ParsedFile],
        batch: usize,
    ) -> Result<BTreeMap<String, i64>, IndexError> {
        let by_path: BTreeMap<&str, &ParsedFile> =
            parsed.iter().map(|p| (p.file.path.as_str(), p)).collect();
        let mut symbols = symbols_by_file(graph);
        let mut inputs: Vec<FileVersionInput> = Vec::with_capacity(graph.files.len());
        for file in &graph.files {
            let info = by_path.get(file.path.as_str());
            inputs.push(FileVersionInput {
                path: file.path.clone(),
                content_hash: info.map_or([0u8; 32], |p| *p.hash.as_bytes()),
                language: info.map_or_else(|| "other".to_owned(), |p| p.language.to_string()),
                analyzer_version: info
                    .map(|p| p.analyzer_version.clone())
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| "none".to_owned()),
                parse_status: info.map_or(FileParseStatus::Skipped, |p| match p.outcome.kind() {
                    FileOutcomeKind::Ok => FileParseStatus::Ok,
                    FileOutcomeKind::Partial => FileParseStatus::Partial,
                    FileOutcomeKind::Failed => FileParseStatus::Failed,
                    FileOutcomeKind::Skipped => FileParseStatus::Skipped,
                }),
                size_bytes: info.map_or(0, |p| u32::try_from(p.file.size).unwrap_or(u32::MAX)),
                diagnostic_count: info.and_then(|p| p.outcome.unit()).map_or(0, |u| {
                    u32::try_from(u.diagnostics.len()).unwrap_or(u32::MAX)
                }),
                symbols: symbols.remove(file.path.as_str()).unwrap_or_default(),
            });
        }
        let mut ids = BTreeMap::new();
        for chunk in inputs.chunks(batch) {
            let refs = self.store.upsert_file_versions(scope, chunk).await?;
            for r in refs {
                ids.insert(r.path, r.id);
            }
        }
        Ok(ids)
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Feeds the link output and the walk results into a [`GraphBuilder`].
fn build_graph(
    parsed: &[ParsedFile],
    linked: codegraph::LinkOutput,
) -> Result<CodeGraph, IndexError> {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    let build = |e: codegraph::GraphBuildError| IndexError::Build(e.to_string());
    for p in parsed {
        if matches!(p.outcome, FileOutcome::Skipped { .. }) {
            continue;
        }
        builder
            .add_file(FileInput {
                path: p.file.path.clone(),
                file_version_id: None,
                content_hash: p.hash,
                language: p.language,
            })
            .map_err(build)?;
    }
    let repo_key = NodeId::repository().key();
    for node in linked.nodes {
        builder.add_node(node).map_err(build)?;
    }
    // Files that failed without a unit (analyzer panic, unreadable) still appear in the graph
    // as a bare file node, so diff mapping can report them as "unanalyzed".
    for p in parsed {
        let bare = matches!(&p.outcome, FileOutcome::Failed { unit: None, .. });
        if !bare {
            continue;
        }
        let path = p.file.path.clone();
        let name = path.as_str().rsplit('/').next().unwrap_or("").to_owned();
        let node = NodeInput::new(NodeId::file(&path), NodeKind::File, name)
            .qualified_name(path.as_str())
            .in_file(path.clone());
        let file_key = node.id.key();
        builder.add_node(node).map_err(build)?;
        builder.add_edge(
            Linker::structural_edge(EdgeKind::Contains, repo_key, file_key).with_origin_file(path),
        );
    }
    for edge in linked.edges {
        builder.add_edge(edge);
    }
    for unresolved in linked.unresolved {
        builder.add_unresolved(unresolved);
    }
    builder.build().map_err(build)
}
