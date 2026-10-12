//! SID-006: the rename/move acceptance suite on `fixtures/repositories/rename-move`.
//!
//! Every scenario directory holds a `base/` and a `head/` tree and an `expected.json`; the harness
//! (`support/lineage_harness.rs`) runs analyzer -> ids -> hashes -> per-file diff -> matcher on
//! real source and compares the outcome. See the fixture README for adding a scenario.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeSet;

use incremental::matcher::{match_symbols, MatcherConfig, RenameHints};
use incremental::symbol_diff::SymbolChangeKind;
use review_core::matcher::{MatchRule, SymbolRef, SymbolTransition};
use serde_json::json;
use support::lineage_harness::{
    assert_scenario, check, load, print_table, run_default, run_scenario, scenario_dirs, summary,
    Scenario,
};

#[test]
fn rename_method_exact_body() {
    let (_, outcome) = assert_scenario("01-rename-method");
    assert_eq!(outcome.result.matches.len(), 1);
    let record = &outcome.result.matches[0];
    assert_eq!(record.rule, MatchRule::ExactBody);
    assert_eq!(record.transition, SymbolTransition::Renamed);
    assert!((record.similarity - 1.0).abs() < f32::EPSILON);
}

#[test]
fn rename_class_members_follow() {
    let (_, outcome) = assert_scenario("02-rename-class");
    assert!(outcome.result.unmatched_added.is_empty());
    assert!(outcome.result.unmatched_removed.is_empty());
    assert!(outcome
        .result
        .matches
        .iter()
        .all(|record| record.transition == SymbolTransition::Renamed));
}

#[test]
fn move_file_all_symbols_moved() {
    let (_, outcome) = assert_scenario("03-move-file");
    assert!(outcome.result.matches.iter().all(|record| {
        record.transition == SymbolTransition::Moved
            && matches!(
                record.rule,
                MatchRule::ExactBody | MatchRule::SignatureAndName
            )
            && record.from_id.as_str().starts_with("ts:src/util/strings#")
            && record.to_id.as_str().starts_with("ts:src/common/strings#")
    }));
}

#[test]
fn move_class_across_files() {
    let (_, outcome) = assert_scenario("04-move-class-across-files");
    assert!(outcome
        .result
        .matches
        .iter()
        .all(|record| record.transition == SymbolTransition::Moved));
}

#[test]
fn rename_plus_small_edit_token_similarity() {
    let (_, outcome) = assert_scenario("05-rename-plus-small-edit");
    let edited = outcome
        .result
        .matches
        .iter()
        .find(|record| record.rule == MatchRule::TokenSimilarity)
        .expect("the edited method pairs by token similarity");
    assert!(edited.similarity >= 0.8 && edited.similarity < 1.0);
}

#[test]
fn rename_plus_large_edit_is_add_and_remove() {
    let (_, outcome) = assert_scenario("05b-rename-plus-large-edit");
    assert!(outcome
        .result
        .matches
        .iter()
        .all(|record| !record.to_id.as_str().contains(".quote/")));
}

#[test]
fn split_file_one_to_one_moves() {
    let (_, outcome) = assert_scenario("06-split-file");
    let functions: Vec<_> = outcome
        .result
        .matches
        .iter()
        .filter(|record| record.to_id.as_str().ends_with("/function"))
        .collect();
    assert_eq!(functions.len(), 5);
    assert!(functions.iter().all(|record| !record.ambiguous));
}

#[test]
fn unrelated_tiny_functions_not_matched() {
    let (_, outcome) = assert_scenario("07-neg-unrelated-tiny-functions");
    assert!(outcome.result.matches.is_empty());
}

#[test]
fn ambiguous_copies_left_unmatched() {
    let (_, outcome) = assert_scenario("08-neg-ambiguous-copies");
    assert!(outcome.result.matches.is_empty());
    assert!(!outcome.result.ambiguous.is_empty());
}

#[test]
fn callers_of_renamed_method_are_body_modifications() {
    let (scenario, outcome) = assert_scenario("09-rename-method-with-callers");
    let lineage_ids: BTreeSet<&str> = outcome
        .result
        .matches
        .iter()
        .flat_map(|record| [record.from_id.as_str(), record.to_id.as_str()])
        .collect();
    assert!(!scenario.expected.modified.is_empty());
    for caller in &scenario.expected.modified {
        assert!(
            !lineage_ids.contains(caller.id.as_str()),
            "{} is a caller, not a rename",
            caller.id
        );
        // The caller keeps its id: it is present on both sides as a modification.
        let change = outcome
            .diffs
            .iter()
            .flat_map(|diff| diff.changes.iter())
            .find(|change| change.id.as_str() == caller.id)
            .expect("the caller is in a diff");
        assert_eq!(change.kind, SymbolChangeKind::Modified);
    }
}

fn all_scenarios() -> Vec<Scenario> {
    scenario_dirs().iter().map(|dir| load(dir)).collect()
}

fn kind_of(pool: &[SymbolRef], id: &str) -> Option<review_core::symbol::SymbolKind> {
    pool.iter()
        .find(|symbol| symbol.id.as_str() == id)
        .map(|symbol| symbol.kind)
}

#[test]
fn lineage_invariants_hold_for_all_scenarios() {
    for scenario in all_scenarios() {
        let outcome = run_default(&scenario);
        let mut from: BTreeSet<&str> = BTreeSet::new();
        let mut to: BTreeSet<&str> = BTreeSet::new();
        for record in &outcome.result.matches {
            assert!(
                from.insert(record.from_id.as_str()),
                "{}: {} matched twice",
                scenario.name,
                record.from_id.as_str()
            );
            assert!(
                to.insert(record.to_id.as_str()),
                "{}: {} matched twice",
                scenario.name,
                record.to_id.as_str()
            );
            let removed_kind = kind_of(&outcome.removed, record.from_id.as_str());
            let added_kind = kind_of(&outcome.added, record.to_id.as_str());
            assert!(
                removed_kind.is_some(),
                "{}: from must exist only in base",
                scenario.name
            );
            assert!(
                added_kind.is_some(),
                "{}: to must exist only in head",
                scenario.name
            );
            assert_eq!(removed_kind, added_kind, "{}: kinds differ", scenario.name);
        }
        print_table(&scenario, &outcome);
    }
}

#[test]
fn scenario_results_independent_of_input_order_and_threads() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    for scenario in all_scenarios() {
        let outcome = run_default(&scenario);
        let reference = summary(&outcome, &outcome.result);

        let mut reversed_removed = outcome.removed.clone();
        let mut reversed_added = outcome.added.clone();
        reversed_removed.reverse();
        reversed_added.reverse();
        let mut rotated_removed = outcome.removed.clone();
        let mut rotated_added = outcome.added.clone();
        let half_removed = rotated_removed.len() / 2;
        rotated_removed.rotate_left(half_removed);
        let half_added = rotated_added.len() / 2;
        rotated_added.rotate_left(half_added);
        for (removed, added) in [
            (&reversed_removed, &reversed_added),
            (&rotated_removed, &rotated_added),
        ] {
            let result = match_symbols(removed, added, &cfg, &hints);
            assert_eq!(
                summary(&outcome, &result),
                reference,
                "{}: input order changed the outcome",
                scenario.name
            );
        }

        let threaded: Vec<serde_json::Value> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let result = match_symbols(&outcome.removed, &outcome.added, &cfg, &hints);
                        summary(&outcome, &result)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("matcher thread"))
                .collect()
        });
        for value in threaded {
            assert_eq!(value, reference, "{}: threads disagree", scenario.name);
        }
    }
}

#[test]
fn every_scenario_dir_has_expected_json() {
    let dirs = scenario_dirs();
    let names: Vec<String> = dirs
        .iter()
        .filter_map(|dir| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    for required in [
        "01-rename-method",
        "02-rename-class",
        "03-move-file",
        "04-move-class-across-files",
        "05-rename-plus-small-edit",
        "05b-rename-plus-large-edit",
        "06-split-file",
        "07-neg-unrelated-tiny-functions",
        "08-neg-ambiguous-copies",
        "09-rename-method-with-callers",
    ] {
        assert!(names.iter().any(|n| n == required), "missing {required}");
    }
    for dir in dirs {
        assert!(dir.join("expected.json").is_file(), "{}", dir.display());
        assert!(dir.join("base").is_dir(), "{}", dir.display());
        assert!(dir.join("head").is_dir(), "{}", dir.display());
    }
}

/// The documented mutation check of SID-006's acceptance criteria: a Jaccard threshold of 0.5
/// pairs the heavily rewritten method of `05b`, and 0.95 rejects the small edit of `05`, so each
/// mutation fails at least one scenario.
#[test]
fn jaccard_threshold_mutations_are_caught() {
    let scenarios = all_scenarios();
    for threshold in [0.5_f32, 0.95] {
        let cfg = MatcherConfig {
            jaccard_min: threshold,
            ..MatcherConfig::default()
        };
        let failing: Vec<&str> = scenarios
            .iter()
            .filter(|scenario| !check(scenario, &run_scenario(scenario, &cfg)).is_empty())
            .map(|scenario| scenario.name.as_str())
            .collect();
        println!("jaccard_min = {threshold}: failing scenarios {failing:?}");
        assert!(
            !failing.is_empty(),
            "a Jaccard threshold of {threshold} must fail at least one scenario"
        );
    }
}

/// Writes `target/rename-move-report.json` for SID-005's calibration report.
#[test]
fn write_rename_move_report() {
    let mut report = Vec::new();
    for scenario in all_scenarios() {
        let outcome = run_default(&scenario);
        report.push(json!({
            "scenario": scenario.name,
            "passed": check(&scenario, &outcome).is_empty(),
            "matches": outcome.result.matches.iter().map(|record| json!({
                "from": record.from_id.as_str(),
                "to": record.to_id.as_str(),
                "rule": record.rule.as_str(),
                "transition": record.transition.as_str(),
                "similarity": record.similarity,
            })).collect::<Vec<_>>(),
        }));
    }
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    if std::fs::create_dir_all(&target).is_ok() {
        let _ = std::fs::write(
            target.join("rename-move-report.json"),
            serde_json::to_string_pretty(&report).unwrap_or_default(),
        );
    }
}
