//! End-to-end lineage harness (SID-006): analyze a scenario's `base/` and `head/` trees with the
//! TypeScript analyzer, diff every file (SID-004), pool the removed and added symbols and run the
//! rename/move matcher (SID-005), then compare the outcome with the scenario's `expected.json`.
//!
//! Reusable by INC-012 and IDX-006: [`load`] and [`run_scenario`] only need a directory with
//! `base/` and `head/` trees.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use analysis_ir::ParsedUnit;
use incremental::matcher::{match_symbols, pools, MatcherConfig, RenameHints};
use incremental::symbol_diff::{diff_units, FileSymbolDiff, SymbolChangeKind};
use review_core::ids::SymbolKey;
use review_core::matcher::{MatchResult, SymbolRef};
use serde::Deserialize;
use serde_json::{json, Value};

use super::analyze;

/// One expected lineage record.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedMatch {
    pub from: String,
    pub to: String,
    pub transition: String,
    pub rule: String,
    #[serde(default)]
    pub min_similarity: Option<f32>,
    /// Exclusive upper bound, for fuzzy matches that must not be exact.
    #[serde(default)]
    pub max_similarity: Option<f32>,
}

/// A symbol the per-file diff must report as `Modified` with exactly these flags.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedModified {
    pub id: String,
    pub flags: Vec<String>,
}

/// The contents of a scenario's `expected.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Expected {
    pub description: String,
    pub matches: Vec<ExpectedMatch>,
    pub unmatched_added: Vec<String>,
    pub unmatched_removed: Vec<String>,
    /// Symbols that must be `Unchanged` (a subset; not every unchanged symbol is listed).
    #[serde(default)]
    pub unchanged: Vec<String>,
    /// Symbols that must be `Modified` with the given flags (a subset).
    #[serde(default)]
    pub modified: Vec<ExpectedModified>,
    pub ambiguous: usize,
}

/// A loaded scenario.
#[derive(Debug)]
pub struct Scenario {
    pub name: String,
    pub dir: PathBuf,
    /// `(repository-relative path, unit)` for every file of `base/`, sorted by path.
    pub base: Vec<(String, ParsedUnit)>,
    /// The same for `head/`.
    pub head: Vec<(String, ParsedUnit)>,
    pub expected: Expected,
}

/// What the analyzer -> diff -> matcher chain produced.
#[derive(Debug)]
pub struct ScenarioOutcome {
    pub diffs: Vec<FileSymbolDiff>,
    pub removed: Vec<SymbolRef>,
    pub added: Vec<SymbolRef>,
    pub result: MatchResult,
}

/// `fixtures/repositories/rename-move`.
pub fn scenarios_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/repositories/rename-move")
}

/// Every scenario directory (one holding `base/` or `expected.json`), sorted by name. The
/// `steps/` history fixture of the same repository is not a scenario.
pub fn scenario_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(scenarios_root())
        .expect("the rename-move fixture exists")
        .map(|entry| entry.expect("readable fixture dir").path())
        .filter(|path| path.is_dir())
        .filter(|path| path.file_name().is_some_and(|name| name != "steps"))
        .collect();
    out.sort();
    out
}

/// The scenario directory with this name.
pub fn scenario_dir(name: &str) -> PathBuf {
    scenarios_root().join(name)
}

fn collect_sources(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("readable entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_sources(root, &path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "ts" || ext == "tsx")
        {
            let relative = path
                .strip_prefix(root)
                .expect("inside the tree")
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let text = std::fs::read_to_string(&path).expect("utf-8 fixture source");
            // Fixtures are LF; normalize in case a checkout converted them.
            out.push((relative, text.replace("\r\n", "\n")));
        }
    }
}

fn analyze_tree(root: &Path) -> Vec<(String, ParsedUnit)> {
    let mut sources = Vec::new();
    if root.is_dir() {
        collect_sources(root, root, &mut sources);
    }
    sources
        .into_iter()
        .map(|(path, text)| {
            let unit = analyze(&path, &text);
            (path, unit)
        })
        .collect()
}

/// Loads and analyzes one scenario.
pub fn load(dir: &Path) -> Scenario {
    let name = dir
        .file_name()
        .expect("scenario dir has a name")
        .to_string_lossy()
        .into_owned();
    let expected_path = dir.join("expected.json");
    let expected: Expected = serde_json::from_str(
        &std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|error| panic!("{}: {error}", expected_path.display())),
    )
    .unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
    Scenario {
        name,
        dir: dir.to_path_buf(),
        base: analyze_tree(&dir.join("base")),
        head: analyze_tree(&dir.join("head")),
        expected,
    }
}

/// Loads a scenario by directory name.
pub fn load_named(name: &str) -> Scenario {
    load(&scenario_dir(name))
}

/// Runs the chain with an explicit configuration.
pub fn run_scenario(scenario: &Scenario, cfg: &MatcherConfig) -> ScenarioOutcome {
    let paths: BTreeSet<&str> = scenario
        .base
        .iter()
        .chain(scenario.head.iter())
        .map(|(path, _)| path.as_str())
        .collect();
    let diffs: Vec<FileSymbolDiff> = paths
        .iter()
        .map(|path| {
            let base = scenario
                .base
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, u)| u);
            let head = scenario
                .head
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, u)| u);
            diff_units(base, head)
        })
        .collect();
    let (removed, added) = pools(
        scenario.base.iter().map(|(_, unit)| unit),
        scenario.head.iter().map(|(_, unit)| unit),
    );
    let result = match_symbols(&removed, &added, cfg, &RenameHints::default());
    ScenarioOutcome {
        diffs,
        removed,
        added,
        result,
    }
}

/// Runs the chain with the default configuration.
pub fn run_default(scenario: &Scenario) -> ScenarioOutcome {
    run_scenario(scenario, &MatcherConfig::default())
}

fn id_by_key(outcome: &ScenarioOutcome) -> BTreeMap<SymbolKey, String> {
    outcome
        .removed
        .iter()
        .chain(outcome.added.iter())
        .map(|symbol| (symbol.key, symbol.id.as_str().to_owned()))
        .collect()
}

fn ids_of(keys: &[SymbolKey], ids: &BTreeMap<SymbolKey, String>) -> BTreeSet<String> {
    keys.iter()
        .map(|key| ids.get(key).cloned().unwrap_or_else(|| format!("{key:?}")))
        .collect()
}

/// A normalized, order-free summary of a match result, for equality across runs.
pub fn summary(outcome: &ScenarioOutcome, result: &MatchResult) -> Value {
    let ids = id_by_key(outcome);
    let mut matches: Vec<(String, String, String, String, String)> = result
        .matches
        .iter()
        .map(|record| {
            (
                record.from_id.as_str().to_owned(),
                record.to_id.as_str().to_owned(),
                record.transition.as_str().to_owned(),
                record.rule.as_str().to_owned(),
                format!("{:.4}", record.similarity),
            )
        })
        .collect();
    matches.sort();
    let mut ambiguous: Vec<String> = result
        .ambiguous
        .iter()
        .map(|note| note.id.as_str().to_owned())
        .collect();
    ambiguous.sort();
    json!({
        "matches": matches,
        "unmatched_added": ids_of(&result.unmatched_added, &ids),
        "unmatched_removed": ids_of(&result.unmatched_removed, &ids),
        "ambiguous": ambiguous,
    })
}

/// The actual outcome in `expected.json` shape, printed when a scenario fails so the difference
/// is readable (and so a new scenario can be bootstrapped from it after review).
pub fn actual_json(outcome: &ScenarioOutcome) -> Value {
    let ids = id_by_key(outcome);
    let matches: Vec<Value> = outcome
        .result
        .matches
        .iter()
        .map(|record| {
            json!({
                "from": record.from_id.as_str(),
                "to": record.to_id.as_str(),
                "transition": record.transition.as_str(),
                "rule": record.rule.as_str(),
                "similarity": record.similarity,
                "ambiguous": record.ambiguous,
            })
        })
        .collect();
    let mut unchanged: Vec<String> = Vec::new();
    let mut modified: Vec<Value> = Vec::new();
    for diff in &outcome.diffs {
        for change in &diff.changes {
            match change.kind {
                SymbolChangeKind::Unchanged => unchanged.push(change.id.as_str().to_owned()),
                SymbolChangeKind::Modified => modified.push(json!({
                    "id": change.id.as_str(),
                    "flags": change.flags.names(),
                })),
                SymbolChangeKind::Added | SymbolChangeKind::Removed => {}
            }
        }
    }
    json!({
        "matches": matches,
        "unmatched_added": ids_of(&outcome.result.unmatched_added, &ids),
        "unmatched_removed": ids_of(&outcome.result.unmatched_removed, &ids),
        "unchanged": unchanged,
        "modified": modified,
        "ambiguous": outcome.result.ambiguous.len(),
        "ambiguous_ids": outcome.result.ambiguous.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
    })
}

/// Every difference between the outcome and the expectation; empty means the scenario passed.
pub fn check(scenario: &Scenario, outcome: &ScenarioOutcome) -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();
    let ids = id_by_key(outcome);
    let expected = &scenario.expected;

    let actual: BTreeMap<(String, String), (String, String, f32)> = outcome
        .result
        .matches
        .iter()
        .map(|record| {
            (
                (
                    record.from_id.as_str().to_owned(),
                    record.to_id.as_str().to_owned(),
                ),
                (
                    record.transition.as_str().to_owned(),
                    record.rule.as_str().to_owned(),
                    record.similarity,
                ),
            )
        })
        .collect();
    let mut wanted: BTreeSet<(String, String)> = BTreeSet::new();
    for m in &expected.matches {
        let pair = (m.from.clone(), m.to.clone());
        wanted.insert(pair.clone());
        match actual.get(&pair) {
            None => problems.push(format!("missing match {} -> {}", m.from, m.to)),
            Some((transition, rule, similarity)) => {
                if transition != &m.transition {
                    problems.push(format!(
                        "{} -> {}: transition {transition}, expected {}",
                        m.from, m.to, m.transition
                    ));
                }
                if rule != &m.rule {
                    problems.push(format!(
                        "{} -> {}: rule {rule}, expected {}",
                        m.from, m.to, m.rule
                    ));
                }
                if let Some(min) = m.min_similarity {
                    if *similarity < min {
                        problems.push(format!(
                            "{} -> {}: similarity {similarity} below {min}",
                            m.from, m.to
                        ));
                    }
                }
                if let Some(max) = m.max_similarity {
                    if *similarity >= max {
                        problems.push(format!(
                            "{} -> {}: similarity {similarity} not below {max}",
                            m.from, m.to
                        ));
                    }
                }
            }
        }
    }
    for pair in actual.keys() {
        if !wanted.contains(pair) {
            problems.push(format!("unexpected match {} -> {}", pair.0, pair.1));
        }
    }

    let added = ids_of(&outcome.result.unmatched_added, &ids);
    let want_added: BTreeSet<String> = expected.unmatched_added.iter().cloned().collect();
    if added != want_added {
        problems.push(format!(
            "unmatched_added {added:?}, expected {want_added:?}"
        ));
    }
    let removed = ids_of(&outcome.result.unmatched_removed, &ids);
    let want_removed: BTreeSet<String> = expected.unmatched_removed.iter().cloned().collect();
    if removed != want_removed {
        problems.push(format!(
            "unmatched_removed {removed:?}, expected {want_removed:?}"
        ));
    }
    if outcome.result.ambiguous.len() != expected.ambiguous {
        problems.push(format!(
            "{} ambiguity notes, expected {}",
            outcome.result.ambiguous.len(),
            expected.ambiguous
        ));
    }

    let changes: BTreeMap<&str, (SymbolChangeKind, Vec<&'static str>)> = outcome
        .diffs
        .iter()
        .flat_map(|diff| diff.changes.iter())
        .map(|change| (change.id.as_str(), (change.kind, change.flags.names())))
        .collect();
    for id in &expected.unchanged {
        match changes.get(id.as_str()) {
            Some((SymbolChangeKind::Unchanged, _)) => {}
            other => problems.push(format!("{id} should be unchanged, got {other:?}")),
        }
    }
    for m in &expected.modified {
        match changes.get(m.id.as_str()) {
            Some((SymbolChangeKind::Modified, flags))
                if flags.iter().copied().eq(m.flags.iter().map(String::as_str)) => {}
            other => problems.push(format!(
                "{} should be modified {:?}, got {other:?}",
                m.id, m.flags
            )),
        }
    }
    problems
}

/// Asserts a scenario passes, printing expected vs actual on failure.
pub fn assert_scenario(name: &str) -> (Scenario, ScenarioOutcome) {
    let scenario = load_named(name);
    let outcome = run_default(&scenario);
    let problems = check(&scenario, &outcome);
    if !problems.is_empty() {
        let mut message = format!(
            "scenario {name} ({}) failed:\n",
            scenario.expected.description
        );
        for problem in &problems {
            let _ = writeln!(message, "  - {problem}");
        }
        let _ = writeln!(
            message,
            "actual outcome:\n{}",
            serde_json::to_string_pretty(&actual_json(&outcome)).unwrap_or_default()
        );
        panic!("{message}");
    }
    print_table(&scenario, &outcome);
    (scenario, outcome)
}

/// The per-scenario summary table (rule, similarity, transition).
pub fn print_table(scenario: &Scenario, outcome: &ScenarioOutcome) {
    println!("scenario {}:", scenario.name);
    for record in &outcome.result.matches {
        println!(
            "  {:<18} {:.3} {:<14} {} -> {}",
            record.rule.as_str(),
            record.similarity,
            record.transition.as_str(),
            record.from_id.as_str(),
            record.to_id.as_str()
        );
    }
    println!(
        "  unmatched: {} added, {} removed; ambiguous: {}",
        outcome.result.unmatched_added.len(),
        outcome.result.unmatched_removed.len(),
        outcome.result.ambiguous.len()
    );
}
