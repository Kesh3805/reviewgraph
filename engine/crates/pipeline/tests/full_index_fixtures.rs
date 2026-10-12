//! Full index pipeline tests (IDX-001, IDX-003, IDX-005) against the in-memory store.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use analysis_ir::traits::{AnalyzeError, LanguageAnalyzer};
use analysis_ir::{AnalyzerConfig, AnalyzerId, AnalyzerRegistry, ParsedUnit, SourceInput};
use async_trait::async_trait;
use graph_storage::kinds::EdgeKind;
use graph_storage::mem::MemGraphStore;
use graph_storage::status::SnapshotStatus;
use graph_storage::types::SnapshotPurpose;
use graph_storage::GraphStore;
use pipeline::index::{
    typescript_registry, FullIndexRequest, FullIndexer, InMemoryParseCache, IndexConfig,
    IndexError, IndexPhase, ProgressSink, SourceSpec, TolerancePolicy,
};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, OrganizationId, RepositoryId, SnapshotId};
use review_core::language::Language;
use review_core::location::RepoPath;
use tokio_util::sync::CancellationToken;

fn scope() -> RepoScope {
    RepoScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

fn commit() -> CommitSha {
    "a".repeat(40).parse().expect("valid sha")
}

fn write(root: &Path, rel: &str, body: &str) {
    let full = root.join(rel);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    std::fs::write(full, body).expect("write");
}

fn small_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "src/a.ts",
        "export function helper(): number { return 1; }\n",
    );
    write(
        dir.path(),
        "src/b.ts",
        "import { helper } from './a';\nexport function main() { return helper(); }\n",
    );
    dir
}

fn request(scope: RepoScope, root: &Path) -> FullIndexRequest {
    FullIndexRequest::new(
        scope,
        commit(),
        SourceSpec::WorkingTree {
            root: root.to_path_buf(),
        },
        SnapshotPurpose::Local,
    )
}

fn indexer(store: Arc<dyn GraphStore>, threads: usize) -> FullIndexer {
    let (registry, parsers) = typescript_registry();
    FullIndexer::new(store, registry, threads)
        .expect("pool")
        .with_parser_versions(parsers)
}

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/repositories")
        .join(rel)
}

#[tokio::test]
async fn indexes_a_small_repository_into_a_ready_snapshot() {
    let repo = small_repo();
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let out = indexer(Arc::clone(&store), 2)
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect("index");
    assert!(!out.reused);
    assert_eq!(out.snapshot.status, SnapshotStatus::Ready);
    assert_eq!(out.stats.files_ok, 2);
    assert!(out.stats.nodes > 0);
    let g = store
        .load_graph(&scope, out.snapshot.id)
        .await
        .expect("load");
    assert_eq!(g.files.len(), 2);
    assert!(g.files.iter().all(|f| f.file_version_id.is_some()));
    assert!(g.edges.iter().any(|e| e.kind == EdgeKind::Imports));
    assert!(!out.snapshot.versions.analyzer_versions.is_empty());
    assert!(out
        .snapshot
        .versions
        .analyzer_versions
        .contains_key("linker"));
    assert_ne!(out.snapshot.versions.fingerprint, [0u8; 32]);
}

#[tokio::test]
async fn indexes_the_ts_basic_calls_fixture() {
    let root = fixture("ts-basic-calls/steps/001-initial");
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let out = indexer(Arc::clone(&store), 2)
        .run(request(scope, &root), &pipeline::index::NoopProgress)
        .await
        .expect("index");
    assert_eq!(out.snapshot.status, SnapshotStatus::Ready);
    assert!(out.stats.files_ok > 0);
    assert_eq!(out.stats.files_failed, 0);
}

#[tokio::test]
async fn full_index_is_deterministic_across_thread_counts() {
    let repo = small_repo();
    let mut loaded = Vec::new();
    for threads in [1usize, 4] {
        let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
        let scope = scope();
        let out = indexer(Arc::clone(&store), threads)
            .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
            .await
            .expect("index");
        loaded.push(
            store
                .load_graph(&scope, out.snapshot.id)
                .await
                .expect("load"),
        );
    }
    assert_eq!(loaded[0], loaded[1]);
}

#[tokio::test]
async fn existing_ready_fingerprint_short_circuits() {
    let repo = small_repo();
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let indexer = indexer(Arc::clone(&store), 2);
    let first = indexer
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect("first");
    let again = indexer
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect("second");
    assert!(again.reused);
    assert_eq!(again.snapshot.id, first.snapshot.id);

    let mut rebuild = request(scope, repo.path());
    rebuild.rebuild = true;
    let rebuilt = indexer
        .run(rebuild, &pipeline::index::NoopProgress)
        .await
        .expect("rebuild");
    assert!(!rebuilt.reused);
    assert_ne!(rebuilt.snapshot.id, first.snapshot.id);
}

#[derive(Default)]
struct CancelOnParse {
    token: CancellationToken,
    snapshot: Mutex<Option<SnapshotId>>,
}

#[async_trait]
impl ProgressSink for CancelOnParse {
    async fn phase(&self, phase: IndexPhase) {
        if phase == IndexPhase::Parsing {
            self.token.cancel();
        }
    }
    async fn files(&self, _done: u64, _total: u64) {}
    async fn started(&self, snapshot: SnapshotId) {
        *self.snapshot.lock().expect("lock") = Some(snapshot);
    }
}

#[tokio::test]
async fn cancellation_marks_snapshot_failed() {
    let repo = small_repo();
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let sink = CancelOnParse::default();
    let mut req = request(scope, repo.path());
    req.cancel = sink.token.clone();
    let err = indexer(Arc::clone(&store), 2)
        .run(req, &sink)
        .await
        .expect_err("cancelled");
    assert!(matches!(err, IndexError::Cancelled));
    let id = sink
        .snapshot
        .lock()
        .expect("lock")
        .expect("snapshot created");
    let meta = store
        .snapshot(&scope, id)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(meta.status, SnapshotStatus::Failed);
    assert!(meta.error.as_deref().unwrap_or("").contains("cancelled"));
}

#[tokio::test]
async fn oversized_and_binary_files_are_skipped() {
    let repo = small_repo();
    write(repo.path(), "src/big.ts", &"a".repeat(4096));
    std::fs::write(repo.path().join("src/bin.ts"), b"export\0const").expect("write");
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let mut req = request(scope, repo.path());
    req.config = IndexConfig {
        max_file_bytes: 1024,
        ..IndexConfig::default()
    };
    let out = indexer(Arc::clone(&store), 2)
        .run(req, &pipeline::index::NoopProgress)
        .await
        .expect("index");
    assert_eq!(out.stats.files_skipped, 2);
    assert_eq!(out.stats.files_ok, 2);
    let g = store
        .load_graph(&scope, out.snapshot.id)
        .await
        .expect("load");
    assert!(g
        .files
        .iter()
        .all(|f| !f.path.ends_with("big.ts") && !f.path.ends_with("bin.ts")));
}

#[tokio::test]
async fn syntax_error_file_is_partial_and_linked() {
    let repo = small_repo();
    write(repo.path(), "src/broken.ts", "export function ( {{{ \n");
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let out = indexer(Arc::clone(&store), 2)
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect("index");
    assert_eq!(out.stats.files_partial, 1);
    let g = store
        .load_graph(&scope, out.snapshot.id)
        .await
        .expect("load");
    assert!(g.files.iter().any(|f| f.path == "src/broken.ts"));
}

#[derive(Debug)]
struct PanicsOnPoison;

impl LanguageAnalyzer for PanicsOnPoison {
    fn id(&self) -> AnalyzerId {
        AnalyzerId {
            name: "poison-test".to_owned(),
            version: "1.0.0".parse().expect("version"),
        }
    }
    fn language(&self) -> Language {
        Language::Typescript
    }
    fn supports(&self, path: &RepoPath) -> bool {
        path.as_str().ends_with(".ts")
    }
    #[allow(clippy::panic)]
    fn analyze(
        &self,
        _input: &SourceInput<'_>,
        _cfg: &AnalyzerConfig,
    ) -> Result<ParsedUnit, AnalyzeError> {
        panic!("poisoned input");
    }
}

fn panicking_indexer(store: Arc<dyn GraphStore>, policy: TolerancePolicy) -> FullIndexer {
    let mut registry = AnalyzerRegistry::new();
    registry.register(Arc::new(PanicsOnPoison));
    FullIndexer::new(store, registry, 2)
        .expect("pool")
        .with_tolerance(policy)
}

#[tokio::test]
async fn analyzer_panic_fails_the_file_but_keeps_a_file_node() {
    let repo = small_repo();
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let policy = TolerancePolicy {
        max_failed_ratio: 1.0,
        max_failed_files: None,
        min_parsed_files: 0,
    };
    let out = panicking_indexer(Arc::clone(&store), policy)
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect("index tolerates failed files");
    assert_eq!(out.stats.files_failed, 2);
    assert_eq!(out.stats.failed_paths_sample, vec!["src/a.ts", "src/b.ts"]);
    let g = store
        .load_graph(&scope, out.snapshot.id)
        .await
        .expect("load");
    assert_eq!(g.files.len(), 2);
}

#[tokio::test]
async fn tolerance_exceeded_fails_index_with_counts() {
    let repo = small_repo();
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let err = panicking_indexer(Arc::clone(&store), TolerancePolicy::default())
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect_err("too many failures");
    match err {
        IndexError::ToleranceExceeded(e) => {
            assert_eq!(e.failed, 2);
            assert_eq!(e.total, 2);
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[tokio::test]
async fn second_index_with_a_parse_cache_parses_zero_files() {
    let repo = small_repo();
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let cache = Arc::new(InMemoryParseCache::new());
    let (registry, parsers) = typescript_registry();
    let indexer = FullIndexer::new(Arc::clone(&store), registry, 2)
        .expect("pool")
        .with_parser_versions(parsers)
        .with_parse_cache(cache.clone());
    let first = indexer
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect("first");
    assert_eq!(first.stats.files_parsed, 2);
    let mut rebuild = request(scope, repo.path());
    rebuild.rebuild = true;
    let second = indexer
        .run(rebuild, &pipeline::index::NoopProgress)
        .await
        .expect("second");
    assert_eq!(second.stats.files_parsed, 0);
    assert_eq!(second.stats.cache_hits, 2);
    assert!(cache.hits() >= 2);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_outside_repo_not_followed() {
    let repo = small_repo();
    let outside = tempfile::tempdir().expect("outside");
    write(outside.path(), "secret.ts", "export const secret = 1;\n");
    std::os::unix::fs::symlink(
        outside.path().join("secret.ts"),
        repo.path().join("src/link.ts"),
    )
    .expect("symlink");
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let scope = scope();
    let out = indexer(Arc::clone(&store), 2)
        .run(request(scope, repo.path()), &pipeline::index::NoopProgress)
        .await
        .expect("index");
    let g = store
        .load_graph(&scope, out.snapshot.id)
        .await
        .expect("load");
    assert!(g.files.iter().all(|f| f.path != "src/link.ts"));
}

#[tokio::test]
async fn syntax_errors_persist_diagnostics_without_source_text() {
    let repo = small_repo();
    write(repo.path(), "src/broken.ts", "export function ( {{{ \n");
    let store: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let diagnostics = Arc::new(graph_storage::diagnostics::MemParseDiagnostics::new());
    let (registry, parsers) = typescript_registry();
    let out = FullIndexer::new(Arc::clone(&store), registry, 2)
        .expect("pool")
        .with_parser_versions(parsers)
        .with_diagnostics(diagnostics.clone())
        .run(
            request(scope(), repo.path()),
            &pipeline::index::NoopProgress,
        )
        .await
        .expect("index");
    assert!(out.stats.diagnostics_written > 0);
    assert_eq!(diagnostics.file_versions(), 1);
}
