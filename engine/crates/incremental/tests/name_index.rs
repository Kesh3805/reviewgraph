#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! INC-004 acceptance tests: the base index, the delta computed from a symbol change set and the
//! overlay that merges them.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use codegraph::{Linker, NameIndex, NameLookup};
use incremental::matcher::RenameHints;
use incremental::metrics::IncrementalCounters;
use incremental::name_index::{BaseIndexes, NameDelta, OverlayNameIndex};
use incremental::symbols::{diff_changed_files, FileChangeInput};
use proptest::prelude::*;
use review_core::matcher::MatcherConfig;
use support::analyze;
use support::inc::{full_graph, repo};

const POOL: usize = 6;

/// One generated file: top-level function indices and method indices of its class.
#[derive(Debug, Clone, Default)]
struct GenFile {
    funcs: BTreeSet<usize>,
    methods: BTreeSet<usize>,
}

fn source(file: usize, spec: &GenFile) -> String {
    let mut out = String::new();
    for f in &spec.funcs {
        out.push_str(&format!(
            "export function fn{f}(): number {{ return {f}; }}\n"
        ));
    }
    if !spec.methods.is_empty() {
        out.push_str(&format!("export class C{file} {{\n"));
        for m in &spec.methods {
            out.push_str(&format!("  m{m}(): number {{ return {m}; }}\n"));
        }
        out.push_str("}\n");
    }
    out
}

fn files(specs: &[GenFile]) -> BTreeMap<String, String> {
    specs
        .iter()
        .enumerate()
        .map(|(i, spec)| (format!("src/f{i}.ts"), source(i, spec)))
        .collect()
}

fn gen_file() -> impl Strategy<Value = GenFile> {
    (
        proptest::collection::btree_set(0..POOL, 0..4),
        proptest::collection::btree_set(0..POOL, 0..3),
    )
        .prop_map(|(funcs, methods)| GenFile { funcs, methods })
}

fn units(map: &BTreeMap<String, String>) -> Vec<Arc<analysis_ir::ParsedUnit>> {
    map.iter().map(|(p, s)| Arc::new(analyze(p, s))).collect()
}

/// Names the pool can produce, for exhaustive comparison.
fn all_names() -> Vec<String> {
    let mut names = Vec::new();
    for i in 0..POOL {
        names.push(format!("fn{i}"));
        names.push(format!("m{i}"));
        names.push(format!("C{i}"));
    }
    names
}

fn change_set(
    base: &BTreeMap<String, String>,
    head: &BTreeMap<String, String>,
) -> incremental::symbols::SymbolChangeSet {
    let paths: BTreeSet<&String> = base.keys().chain(head.keys()).collect();
    let inputs: Vec<FileChangeInput> = paths
        .into_iter()
        .map(|p| FileChangeInput {
            path: p.parse().unwrap(),
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

fn assert_overlay_matches_rebuilt(
    base: &BTreeMap<String, String>,
    head: &BTreeMap<String, String>,
) {
    let base_graph = full_graph(base);
    let indexes = BaseIndexes::build(&base_graph);
    // The graph-derived index equals the table-derived one the linker used.
    let (_, from_units) = Linker::build_tables(&units(base));
    for name in all_names() {
        assert_eq!(
            indexes.names.top(&name),
            from_units.top(&name),
            "top {name}"
        );
        assert_eq!(
            indexes.names.members(&name),
            from_units.members(&name),
            "members {name}"
        );
    }

    let delta = NameDelta::from_changes(&change_set(base, head));
    let overlay = OverlayNameIndex::new(&indexes.names, &delta);
    let (_, rebuilt) = Linker::build_tables(&units(head));
    for name in all_names() {
        assert_eq!(overlay.top(&name), rebuilt.top(&name), "overlay top {name}");
        assert_eq!(
            overlay.members(&name),
            rebuilt.members(&name),
            "overlay members {name}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn overlay_equals_rebuilt_index_for_every_name(
        base in proptest::collection::vec(gen_file(), 1..5),
        head in proptest::collection::vec(gen_file(), 1..5),
    ) {
        assert_overlay_matches_rebuilt(&files(&base), &files(&head));
    }
}

#[test]
fn unique_to_ambiguous_flagged_threshold_crossed() {
    let base = repo(&[("src/a.ts", "export function dup(): number { return 1; }\n")]);
    let head = repo(&[
        ("src/a.ts", "export function dup(): number { return 1; }\n"),
        ("src/b.ts", "export function dup(): number { return 2; }\n"),
    ]);
    let graph = full_graph(&base);
    let indexes = BaseIndexes::build(&graph);
    let delta = NameDelta::from_changes(&change_set(&base, &head));
    let affected = delta.affected_names(&indexes.names, 3);
    // The IR parents top-level declarations under the module symbol, so the linker indexes them
    // as members; accept either index.
    assert!(affected.top.contains("dup") || affected.members.contains("dup"));
    assert!(affected.threshold_crossed.contains("dup"));
}

#[test]
fn rename_affects_old_and_new_names() {
    let body = "export function oldName(a: number, b: number): number {\n  const s = a + b;\n  return s * 2 + a - b;\n}\n";
    let base = repo(&[("src/a.ts", body)]);
    let head = repo(&[("src/a.ts", &body.replace("oldName", "newName"))]);
    let graph = full_graph(&base);
    let indexes = BaseIndexes::build(&graph);
    let delta = NameDelta::from_changes(&change_set(&base, &head));
    let affected = delta.affected_names(&indexes.names, 3);
    let seen = |n: &str| affected.top.contains(n) || affected.members.contains(n);
    assert!(seen("oldName") && seen("newName"));
}

#[test]
fn modified_in_place_symbols_do_not_touch_the_index() {
    // Golden shape: `authorize` is modified only, so no name's candidate set changes.
    let base = repo(&[(
        "src/a.ts",
        "export function authorize(): number { return 1; }\n",
    )]);
    let head = repo(&[(
        "src/a.ts",
        "export function authorize(): number { return 2; }\n",
    )]);
    let graph = full_graph(&base);
    let indexes = BaseIndexes::build(&graph);
    let delta = NameDelta::from_changes(&change_set(&base, &head));
    assert!(delta.is_empty());
    assert_eq!(
        delta.affected_names(&indexes.names, 3),
        incremental::name_index::AffectedNames::default()
    );
}

#[test]
fn candidates_sorted_and_deduplicated_from_graph() {
    let base = repo(&[
        ("src/a.ts", "export function dup(): number { return 1; }\n"),
        ("src/b.ts", "export function dup(): number { return 2; }\n"),
        ("src/c.ts", "export function dup(): number { return 3; }\n"),
    ]);
    let graph = full_graph(&base);
    let index = NameIndex::from_graph(&graph);
    let slice = if index.top("dup").is_empty() {
        index.members("dup")
    } else {
        index.top("dup")
    };
    assert_eq!(slice.len(), 3);
    let mut sorted = slice.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(slice, sorted.as_slice());
}
