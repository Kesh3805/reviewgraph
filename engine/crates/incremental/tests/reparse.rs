#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! INC-002 acceptance tests: hash-skip and reparse-only-changed.

mod support;

use std::sync::Arc;

use incremental::blob_source::MemBlobSource;
use incremental::changeset::{ChangeSet, ChangeSetStats, PathChange, PathChangeKind};
use incremental::metrics::IncrementalCounters;
use incremental::reparse::{
    reparse_changed, FileOutcome, InMemoryUnitCache, ReparseCx, SkipReason, UnitCache, UnitKey,
    UnitParser,
};
use review_core::location::{ContentHash, RepoPath};
use support::inc::{full_graph, repo, CountingParser};

fn path(raw: &str) -> RepoPath {
    raw.parse().unwrap()
}

fn change(raw: &str, kind: PathChangeKind, from: Option<&str>) -> PathChange {
    PathChange {
        path: path(raw),
        kind,
        renamed_from: from.map(path),
        base_oid: None,
        head_oid: Some("0".repeat(40)),
        similarity: None,
    }
}

fn set(mut entries: Vec<PathChange>) -> ChangeSet {
    entries.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    ChangeSet {
        base: "a".repeat(40).parse().unwrap(),
        head: "b".repeat(40).parse().unwrap(),
        stats: ChangeSetStats {
            entries: entries.len() as u32,
            ..ChangeSetStats::default()
        },
        entries,
    }
}

const A: &str = "export function alpha(): number { return 1; }\n";
const B: &str = "export function beta(): number { return 2; }\n";
const C: &str = "export function gamma(): number { return 3; }\n";

fn blobs(files: &[(&str, &str)]) -> MemBlobSource {
    let mut source = MemBlobSource::new();
    for (p, c) in files {
        source.insert(path(p), c.as_bytes());
    }
    source
}

struct Env {
    graph: codegraph::Graph,
    parser: CountingParser,
    counters: IncrementalCounters,
}

fn env() -> Env {
    let base = repo(&[("src/a.ts", A), ("src/b.ts", B), ("src/c.ts", C)]);
    Env {
        graph: full_graph(&base),
        parser: CountingParser::new(),
        counters: IncrementalCounters::new(),
    }
}

fn run(
    env: &Env,
    blobs: &MemBlobSource,
    cache: Option<&dyn UnitCache>,
    changes: &ChangeSet,
) -> incremental::reparse::ReparseResult {
    let cx = ReparseCx {
        base: &env.graph,
        blobs,
        parser: &env.parser,
        cache,
        max_file_bytes: 1024,
        counters: &env.counters,
    };
    reparse_changed(&cx, changes).unwrap()
}

#[test]
fn identical_content_is_skipped_without_analyzer_call() {
    let env = env();
    let blobs = blobs(&[("src/a.ts", A)]);
    let result = run(
        &env,
        &blobs,
        None,
        &set(vec![change("src/a.ts", PathChangeKind::Modified, None)]),
    );
    assert!(matches!(
        result.outcomes.as_slice(),
        [FileOutcome::Unchanged { .. }]
    ));
    assert_eq!(env.parser.calls(), 0);
    assert_eq!(result.counters.unchanged, 1);
    assert_eq!(env.counters.snapshot().files_skipped_unchanged_total, 1);
    assert_eq!(env.counters.snapshot().files_reparsed_total, 0);
}

#[test]
fn modified_file_is_parsed_once() {
    let env = env();
    let edited = "export function alpha(): number { return 100; }\n";
    let blobs = blobs(&[("src/a.ts", edited)]);
    let result = run(
        &env,
        &blobs,
        None,
        &set(vec![change("src/a.ts", PathChangeKind::Modified, None)]),
    );
    assert_eq!(env.parser.calls(), 1);
    match result.outcomes.as_slice() {
        [FileOutcome::Reparsed {
            unit,
            cache_hit,
            base_hash,
            ..
        }] => {
            assert!(!cache_hit);
            assert_eq!(unit.content_hash, ContentHash::of(edited.as_bytes()));
            assert_eq!(*base_hash, Some(ContentHash::of(A.as_bytes())));
        }
        other => panic!("unexpected outcomes {other:?}"),
    }
    assert_eq!(env.counters.snapshot().files_reparsed_total, 1);
    assert_eq!(env.counters.snapshot().files_reparsed_analyzer_total, 1);
}

#[test]
fn revert_to_base_content_is_skipped() {
    // A path that appears in the diff but whose head content equals the base content.
    let env = env();
    let blobs = blobs(&[("src/b.ts", B), ("src/c.ts", C)]);
    let result = run(
        &env,
        &blobs,
        None,
        &set(vec![
            change("src/b.ts", PathChangeKind::Modified, None),
            change("src/c.ts", PathChangeKind::Modified, None),
        ]),
    );
    assert_eq!(env.parser.calls(), 0);
    assert_eq!(result.counters.unchanged, 2);
}

#[test]
fn rename_only_is_reparsed_as_new_path_unless_cached() {
    let env = env();
    let blobs = blobs(&[("src/moved.ts", A)]);
    let changes = set(vec![change(
        "src/moved.ts",
        PathChangeKind::Renamed,
        Some("src/a.ts"),
    )]);
    let result = run(&env, &blobs, None, &changes);
    assert_eq!(env.parser.calls(), 1);
    let paths: Vec<&str> = result.outcomes.iter().map(|o| o.path().as_str()).collect();
    assert_eq!(paths, vec!["src/a.ts", "src/moved.ts"]);
    assert!(matches!(result.outcomes[0], FileOutcome::Deleted { .. }));
    assert!(matches!(result.outcomes[1], FileOutcome::Reparsed { .. }));

    // With the unit already cached under the new path, the analyzer is not called again.
    let cache = InMemoryUnitCache::new();
    let unit = Arc::new(support::analyze("src/moved.ts", A));
    cache.store(
        UnitKey {
            path: path("src/moved.ts"),
            content_hash: ContentHash::of(A.as_bytes()),
            analyzer_version: env.parser.analyzer_version(&path("src/moved.ts")),
        },
        unit,
    );
    let before = env.parser.calls();
    let cached = run(&env, &blobs, Some(&cache), &changes);
    assert_eq!(env.parser.calls(), before);
    assert_eq!(cached.counters.cache_hits, 1);
}

#[test]
fn cache_hit_skips_analyzer() {
    let env = env();
    let edited = "export function alpha(): number { return 7; }\n";
    let blobs = blobs(&[("src/a.ts", edited)]);
    let cache = InMemoryUnitCache::new();
    let changes = set(vec![change("src/a.ts", PathChangeKind::Modified, None)]);
    let first = run(&env, &blobs, Some(&cache), &changes);
    assert_eq!(first.counters.parsed, 1);
    assert_eq!(cache.len(), 1);
    let second = run(&env, &blobs, Some(&cache), &changes);
    assert_eq!(second.counters.cache_hits, 1);
    assert_eq!(second.counters.parsed, 0);
    assert_eq!(env.parser.calls(), 1);
}

#[test]
fn deleted_file_not_read() {
    let env = env();
    let blobs = blobs(&[]);
    let result = run(
        &env,
        &blobs,
        None,
        &set(vec![change("src/c.ts", PathChangeKind::Deleted, None)]),
    );
    assert!(blobs.reads().is_empty());
    assert_eq!(result.deleted(), vec![path("src/c.ts")]);
}

#[test]
fn oversize_and_binary_skipped() {
    let env = env();
    let big = "x".repeat(2000);
    let mut source = MemBlobSource::new();
    source.insert(path("src/big.ts"), big.as_bytes());
    source.insert(path("src/bin.ts"), vec![0u8, 1, 2, 3]);
    let result = run(
        &env,
        &source,
        None,
        &set(vec![
            change("src/big.ts", PathChangeKind::Added, None),
            change("src/bin.ts", PathChangeKind::Added, None),
        ]),
    );
    let reasons: Vec<SkipReason> = result
        .outcomes
        .iter()
        .map(|o| match o {
            FileOutcome::Skipped { reason, .. } => *reason,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(reasons, vec![SkipReason::Binary, SkipReason::TooLarge]);
    assert_eq!(env.parser.calls(), 0);
}

#[test]
fn unchanged_files_in_tree_never_read() {
    let env = env();
    let edited = "export function beta(): number { return 22; }\n";
    let blobs = blobs(&[("src/a.ts", A), ("src/b.ts", edited), ("src/c.ts", C)]);
    let _ = run(
        &env,
        &blobs,
        None,
        &set(vec![change("src/b.ts", PathChangeKind::Modified, None)]),
    );
    assert_eq!(blobs.reads(), vec![path("src/b.ts")]);
    assert_eq!(env.parser.parsed_paths(), vec![path("src/b.ts")]);
}

#[test]
fn partial_parse_unit_is_kept() {
    let env = env();
    let broken = "export function oops( {\n";
    let blobs = blobs(&[("src/new.ts", broken)]);
    let result = run(
        &env,
        &blobs,
        None,
        &set(vec![change("src/new.ts", PathChangeKind::Added, None)]),
    );
    assert!(matches!(
        result.outcomes.as_slice(),
        [FileOutcome::Reparsed { .. }]
    ));
}

#[test]
fn outcomes_sorted_by_path() {
    let env = env();
    let blobs = blobs(&[
        ("src/z.ts", "export const z = 1;\n"),
        ("src/m.ts", "export const m = 1;\n"),
        ("src/a.ts", "export const a = 9;\n"),
    ]);
    let result = run(
        &env,
        &blobs,
        None,
        &set(vec![
            change("src/z.ts", PathChangeKind::Added, None),
            change("src/m.ts", PathChangeKind::Added, None),
            change("src/a.ts", PathChangeKind::Modified, None),
        ]),
    );
    let paths: Vec<&str> = result.outcomes.iter().map(|o| o.path().as_str()).collect();
    assert_eq!(paths, vec!["src/a.ts", "src/m.ts", "src/z.ts"]);
}

#[test]
fn blob_read_failure_fails_the_update() {
    let env = env();
    let blobs = blobs(&[]);
    let cx = ReparseCx {
        base: &env.graph,
        blobs: &blobs,
        parser: &env.parser,
        cache: None,
        max_file_bytes: 1024,
        counters: &env.counters,
    };
    let err = reparse_changed(
        &cx,
        &set(vec![change("src/missing.ts", PathChangeKind::Added, None)]),
    )
    .expect_err("a missing blob must fail the whole update");
    assert!(matches!(err, incremental::Error::BlobRead { .. }));
}
