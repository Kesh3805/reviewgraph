//! SID-004 acceptance: for every paired file of `fixtures/repositories/diff-basic` the per-symbol
//! classification of `diff_units` matches `expected.json` exactly (see the fixture README).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use analysis_ir::ParsedUnit;
use incremental::symbol_diff::{diff_units, SymbolChangeKind};
use serde::Deserialize;
use support::analyze;

#[derive(Debug, Deserialize)]
struct ExpectedModified {
    id: String,
    flags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Expected {
    unchanged: Vec<String>,
    modified: Vec<ExpectedModified>,
    added: Vec<String>,
    removed: Vec<String>,
    moved_range: Vec<String>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/repositories/diff-basic")
}

fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, ParsedUnit>) {
    if !dir.is_dir() {
        return;
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "ts") {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("\r\n", "\n");
            out.insert(relative.clone(), analyze(&relative, &text));
        }
    }
}

fn tree(side: &str) -> BTreeMap<String, ParsedUnit> {
    let dir = root().join(side);
    let mut out = BTreeMap::new();
    collect(&dir, &dir, &mut out);
    out
}

#[test]
fn diff_basic_matches_expected_classification() {
    let expected: Expected = serde_json::from_str(
        &std::fs::read_to_string(root().join("expected.json")).expect("expected.json exists"),
    )
    .expect("expected.json parses");
    let base = tree("base");
    let head = tree("head");
    assert!(
        !base.is_empty() && !head.is_empty(),
        "both trees have files"
    );

    let paths: BTreeSet<&String> = base.keys().chain(head.keys()).collect();
    let mut unchanged = BTreeSet::new();
    let mut modified: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    let mut added = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut moved = BTreeSet::new();
    for path in paths {
        let diff = diff_units(base.get(path), head.get(path));
        assert!(diff.is_known(), "{path}: the diff must be computable");
        for change in &diff.changes {
            let id = change.id.as_str().to_owned();
            if change.moved_range {
                moved.insert(id.clone());
            }
            match change.kind {
                SymbolChangeKind::Unchanged => {
                    unchanged.insert(id);
                }
                SymbolChangeKind::Modified => {
                    modified.insert(id, change.flags.names());
                }
                SymbolChangeKind::Added => {
                    added.insert(id);
                }
                SymbolChangeKind::Removed => {
                    removed.insert(id);
                }
            }
        }
    }

    for id in &expected.unchanged {
        assert!(unchanged.contains(id), "{id} should be Unchanged");
    }
    for id in &expected.added {
        assert!(
            added.contains(id),
            "{id} should be Added; added = {added:?}"
        );
    }
    for id in &expected.removed {
        assert!(
            removed.contains(id),
            "{id} should be Removed; removed = {removed:?}"
        );
    }
    for id in &expected.moved_range {
        assert!(moved.contains(id), "{id} should report moved_range");
    }

    let wanted: BTreeMap<String, Vec<String>> = expected
        .modified
        .iter()
        .map(|entry| (entry.id.clone(), entry.flags.clone()))
        .collect();
    let actual: BTreeMap<String, Vec<String>> = modified
        .iter()
        .map(|(id, flags)| {
            (
                id.clone(),
                flags.iter().map(|flag| (*flag).to_owned()).collect(),
            )
        })
        .collect();
    assert_eq!(
        actual, wanted,
        "the Modified set and flags must match exactly"
    );
}
