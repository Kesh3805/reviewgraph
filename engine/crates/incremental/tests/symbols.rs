#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! INC-003 acceptance tests: symbol classification, rename/move pairing and the head view.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use codegraph::{LineageTransition, NodeId, NodeKey};
use diff_engine::testkit::Scenario;
use incremental::matcher::RenameHints;
use incremental::metrics::IncrementalCounters;
use incremental::symbol_diff::ModifiedFlags;
use incremental::symbols::{
    diff_changed_files, FileChangeInput, HeadSymbolTables, MapUnitSource, MemoSymbolSource,
    SymbolChangeSet,
};
use review_core::location::RepoPath;
use review_core::matcher::MatcherConfig;
use support::analyze;

fn key(id: &str) -> NodeKey {
    NodeId::from_canonical(id).key()
}

fn path(raw: &str) -> RepoPath {
    raw.parse().unwrap()
}

fn diff(base: &[(&str, &str)], head: &[(&str, &str)]) -> SymbolChangeSet {
    let base: BTreeMap<&str, &str> = base.iter().copied().collect();
    let head: BTreeMap<&str, &str> = head.iter().copied().collect();
    let paths: BTreeSet<&str> = base.keys().chain(head.keys()).copied().collect();
    let inputs: Vec<FileChangeInput> = paths
        .into_iter()
        .map(|p| FileChangeInput {
            path: path(p),
            base: base.get(p).map(|s| Arc::new(analyze(p, s))),
            head: head.get(p).map(|s| Arc::new(analyze(p, s))),
        })
        .collect();
    diff_changed_files(
        &inputs,
        &RenameHints::default(),
        &MatcherConfig::default(),
        &IncrementalCounters::new(),
    )
    .unwrap()
}

const BODY_A: &str = "export function compute(a: number, b: number): number {\n  const sum = a + b;\n  const product = a * b;\n  return sum + product;\n}\n";

#[test]
fn body_edit_is_modified_body_only() {
    let head = BODY_A.replace("a * b", "a * b * 2");
    let cs = diff(&[("src/a.ts", BODY_A)], &[("src/a.ts", head.as_str())]);
    assert!(cs.added.is_empty() && cs.removed.is_empty());
    assert_eq!(
        cs.modified,
        vec![(key("ts:src/a#compute/function"), ModifiedFlags::BODY)]
    );
}

#[test]
fn signature_edit_is_modified_not_add_delete() {
    let head = BODY_A.replace("b: number)", "b: number, c: number)");
    let cs = diff(&[("src/a.ts", BODY_A)], &[("src/a.ts", head.as_str())]);
    assert!(cs.added.is_empty() && cs.removed.is_empty());
    assert_eq!(cs.modified.len(), 1);
    assert!(cs.modified[0].1.contains(ModifiedFlags::SIGNATURE));
}

#[test]
fn comment_edit_leaves_symbol_unchanged() {
    let head = BODY_A.replace("const sum", "// a comment\n  const sum");
    let cs = diff(&[("src/a.ts", BODY_A)], &[("src/a.ts", head.as_str())]);
    assert!(cs.modified.is_empty(), "{:?}", cs.modified);
    assert!(cs.added.is_empty() && cs.removed.is_empty());
}

#[test]
fn line_shift_is_moved_range_not_modified() {
    let head = format!("// header\n\n{BODY_A}");
    let cs = diff(&[("src/a.ts", BODY_A)], &[("src/a.ts", head.as_str())]);
    assert!(cs.modified.is_empty());
    assert_eq!(cs.moved_range, vec![key("ts:src/a#compute/function")]);
}

#[test]
fn rename_within_file_detected_by_body_hash() {
    let head = BODY_A.replace("compute", "calculate");
    let cs = diff(&[("src/a.ts", BODY_A)], &[("src/a.ts", head.as_str())]);
    assert_eq!(cs.renamed.len(), 1);
    let record = &cs.renamed[0];
    assert_eq!(record.from, Some(key("ts:src/a#compute/function")));
    assert_eq!(record.to, key("ts:src/a#calculate/function"));
    assert!(matches!(
        record.transition,
        LineageTransition::Renamed { .. }
    ));
    // The pair is one rename for the counters, but both keys still reach the name index.
    let counts = cs.counts();
    assert_eq!((counts.added, counts.removed, counts.renamed), (0, 0, 1));
    assert_eq!(cs.added.len(), 1);
    assert_eq!(cs.removed.len(), 1);
}

#[test]
fn move_across_files_detected() {
    let cs = diff(
        &[
            ("src/a.ts", BODY_A),
            ("src/b.ts", "export const keep = 1;\n"),
        ],
        &[("src/a.ts", ""), ("src/b.ts", BODY_A)],
    );
    let moved: Vec<_> = cs
        .renamed
        .iter()
        .filter(|r| matches!(r.transition, LineageTransition::Moved { .. }))
        .collect();
    assert_eq!(moved.len(), 1, "{:?}", cs.renamed);
    assert_eq!(moved[0].to, key("ts:src/b#compute/function"));
}

#[test]
fn deleted_file_removes_all_symbols() {
    let two = "export function one(): number { return 1; }\nexport function two(): number { return 2; }\n";
    let cs = diff(&[("src/a.ts", two)], &[]);
    assert_eq!(cs.removed.len(), 2);
    assert!(cs.added.is_empty());
    assert!(cs.changed_files.contains(&path("src/a.ts")));
}

#[test]
fn members_are_flagged_for_the_member_index() {
    let source = "export class Svc {\n  run(): number { return 1; }\n}\n";
    let cs = diff(&[], &[("src/s.ts", source)]);
    let by_name: BTreeMap<&str, bool> = cs
        .added
        .iter()
        .map(|r| (r.entry.name.as_str(), r.entry.is_member))
        .collect();
    assert_eq!(by_name.get("Svc"), Some(&false));
    assert_eq!(by_name.get("run"), Some(&true));
}

#[test]
fn base_table_lazy_load_only_for_consulted_files() {
    let units: Vec<Arc<_>> = [
        ("src/a.ts", BODY_A),
        ("src/b.ts", "export const b = 1;\n"),
        ("src/c.ts", "export const c = 1;\n"),
    ]
    .iter()
    .map(|(p, s)| Arc::new(analyze(p, s)))
    .collect();
    let source = Arc::new(MapUnitSource::new(units.iter().cloned()));
    let base = Arc::new(MemoSymbolSource::new(source.clone()));
    let changed = Arc::new(analyze("src/a.ts", &BODY_A.replace("a * b", "a - b")));
    let tables = HeadSymbolTables::new(&[changed], BTreeSet::from([path("src/c.ts")]), base);

    assert!(tables.file_symbols(&path("src/a.ts")).is_some(), "fresh");
    assert!(tables.file_symbols(&path("src/c.ts")).is_none(), "removed");
    assert!(source.reads().is_empty(), "nothing consulted yet");
    assert!(tables.file_symbols(&path("src/b.ts")).is_some());
    assert!(tables.file_symbols(&path("src/b.ts")).is_some());
    assert_eq!(
        source.reads(),
        vec![path("src/b.ts")],
        "loaded once, memoized"
    );
}

#[test]
fn golden_authorize_modified_body() {
    let scenario = Scenario::load("auth-bypass").unwrap();
    let changed = ["src/auth/auth.service.ts", "src/util/format.ts"];
    let inputs: Vec<FileChangeInput> = changed
        .iter()
        .map(|p| FileChangeInput {
            path: path(p),
            base: scenario.base_files.get(*p).map(|s| Arc::new(analyze(p, s))),
            head: scenario.head_files.get(*p).map(|s| Arc::new(analyze(p, s))),
        })
        .collect();
    let cs = diff_changed_files(
        &inputs,
        &RenameHints::default(),
        &MatcherConfig::default(),
        &IncrementalCounters::new(),
    )
    .unwrap();
    assert_eq!(cs.modified.len(), 1, "{:?}", cs.modified);
    assert_eq!(
        cs.modified[0],
        (
            key("ts:src/auth/auth.service#AuthService.authorize/method"),
            ModifiedFlags::BODY
        )
    );
    assert!(cs.added.is_empty() && cs.removed.is_empty());
}
