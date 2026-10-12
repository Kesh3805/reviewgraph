#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Shared helpers of the golden tests (DIFF-007, CHG-008): scenario loading, TypeScript units
//! for changed files, canonical JSON and golden comparison.

use std::path::PathBuf;

use analysis_ir::unit::ParsedUnit;
use analysis_ir::{AnalyzerConfig, LanguageAnalyzer, SourceInput};
use diff_engine::files::{diff_commits, DiffOptions};
use diff_engine::model::DiffModel;
use diff_engine::symbol_map::UnitMap;
use diff_engine::testkit::Scenario;
use lang_typescript::TypeScriptAnalyzer;
use review_core::change::FileChangeStatus;
use review_core::location::{ContentHash, DiffSide, RepoPath};
use review_core::symbol::ModulePath;
use serde_json::{Map, Value};

/// Parse one TypeScript file with the real analyzer.
pub fn analyze(path: &str, src: &str) -> ParsedUnit {
    let repo_path = RepoPath::new(path).unwrap();
    let input = SourceInput {
        module_path: ModulePath::of(&repo_path),
        content_hash: ContentHash::of(src.as_bytes()),
        path: repo_path,
        bytes: src.as_bytes(),
        is_generated: false,
    };
    TypeScriptAnalyzer::new()
        .analyze(&input, &AnalyzerConfig::default())
        .unwrap()
}

fn is_ts(path: &str) -> bool {
    path.ends_with(".ts") || path.ends_with(".tsx") || path.ends_with(".js")
}

/// Units of every changed TypeScript file on both sides.
pub fn units_for(scenario: &Scenario, diff: &DiffModel) -> UnitMap {
    let mut units = UnitMap::default();
    for file in &diff.files {
        let new_path = file.file.path.as_str();
        let old_path = file.file.old_path.as_ref().map_or(new_path, |p| p.as_str());
        if file.file.status != FileChangeStatus::Deleted && is_ts(new_path) {
            if let Some(src) = scenario.head_files.get(new_path) {
                units.insert(DiffSide::Head, analyze(new_path, src));
            }
        }
        if file.file.status != FileChangeStatus::Added && is_ts(old_path) {
            if let Some(src) = scenario.base_files.get(old_path) {
                units.insert(DiffSide::Base, analyze(old_path, src));
            }
        }
    }
    units
}

/// Load a scenario and diff it with default options.
pub fn load(name: &str) -> (Scenario, DiffModel) {
    let scenario = Scenario::load(name).unwrap();
    let diff = diff_scenario(&scenario);
    (scenario, diff)
}

/// Diff an already built scenario.
pub fn diff_scenario(scenario: &Scenario) -> DiffModel {
    let git = scenario.repo.open().unwrap();
    diff_commits(
        &git,
        &scenario.base,
        &scenario.head,
        &DiffOptions::default(),
    )
    .unwrap()
}

/// The value with every object's keys sorted (independent of serde_json's map ordering).
pub fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), canonical(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Pretty canonical JSON text with a trailing newline.
pub fn canonical_text(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&canonical(value)).unwrap();
    text.push('\n');
    text
}

/// Compare `actual` with the committed golden file. On mismatch the full actual JSON is
/// printed so the golden can be reviewed and updated; set `REVIEWGRAPH_BLESS=1` to rewrite it.
pub fn assert_golden(path: PathBuf, actual: &Value) {
    let actual_text = canonical_text(actual);
    if std::env::var_os("REVIEWGRAPH_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual_text).unwrap();
        return;
    }
    let expected_text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}\nactual:\n{actual_text}", path.display()));
    let expected: Value = serde_json::from_str(&expected_text).unwrap();
    if canonical(&expected) != canonical(actual) {
        panic!(
            "golden mismatch for {}\n--- actual (review, then copy or run with REVIEWGRAPH_BLESS=1) ---\n{actual_text}",
            path.display()
        );
    }
}

/// `[[start, end], ...]` for a list of ranges.
pub fn ranges_json(ranges: &[std::ops::Range<u32>]) -> Value {
    Value::Array(
        ranges
            .iter()
            .map(|r| Value::Array(vec![Value::from(r.start), Value::from(r.end)]))
            .collect(),
    )
}
