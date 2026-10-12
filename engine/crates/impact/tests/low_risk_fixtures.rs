//! RISK-006: the low-risk fixtures under `fixtures/pull-requests/` pair a pull request (base
//! tree and patch) with its expected low-risk verdicts. The change set of each is built by hand
//! here until the change-model adapter lands; the expectation files are the contract.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::PathBuf;

use codegraph::NodeKind;
use impact::input::{facts, ChangeSet, FileInput, ImportChange, SymbolInput};
use impact::risk::low_risk::{detect_low_risk, LowRiskContext};
use review_core::change::{FileChangeStatus, SymbolChange};
use review_core::ids::SymbolId;
use serde_json::{json, Value};
use support::{body_change, changed, modified_file, path};

struct Case {
    changes: ChangeSet,
    context: LowRiskContext,
    /// `(file, name)` of every changed symbol, for the expectation file.
    names: Vec<(String, String)>,
}

fn case(symbols: Vec<SymbolInput>, files: Vec<FileInput>) -> Case {
    let names = symbols
        .iter()
        .map(|s| {
            let qualified = s.id().rsplit_once('#').map_or("", |(_, rest)| rest);
            let name = qualified.rsplit_once('/').map_or(qualified, |(n, _)| n);
            (s.path().as_str().to_owned(), name.to_owned())
        })
        .collect();
    Case {
        changes: ChangeSet {
            symbols,
            files,
            ..ChangeSet::default()
        },
        context: LowRiskContext::default(),
        names,
    }
}

fn rename(file: &str, name: &str, from: &str, exported: bool) -> SymbolInput {
    let change = SymbolChange::Renamed {
        from: SymbolId::from_canonical_unchecked(from.to_owned()),
        similarity: 1.0,
    };
    let mut symbol = changed(file, name, NodeKind::Function, change);
    symbol.exported = exported;
    symbol
}

fn build(name: &str) -> Case {
    match name {
        "format-only" => {
            // Only a comment was added: the change model flags the symbol cosmetic.
            let mut greet = changed(
                "src/util/greet.ts",
                "greet",
                NodeKind::Function,
                body_change(),
            );
            greet.cosmetic = true;
            case(vec![greet], vec![modified_file("src/util/greet.ts")])
        }
        "pure-rename" => case(
            vec![
                rename(
                    "src/util/text.ts",
                    "normalizeInput",
                    "ts:src/util/text#trimInput/function",
                    false,
                ),
                changed(
                    "src/util/text.ts",
                    "clean",
                    NodeKind::Function,
                    body_change(),
                ),
            ],
            vec![modified_file("src/util/text.ts")],
        ),
        "exported-rename" => {
            let renamed = rename(
                "src/util/names.ts",
                "formatFullName",
                "ts:src/util/names#formatName/function",
                true,
            );
            let mut built = case(
                vec![renamed.clone()],
                vec![modified_file("src/util/names.ts")],
            );
            // `src/user/profile.ts`, outside the pull request, still calls the old name.
            built
                .context
                .symbols_with_external_callers
                .insert(renamed.key());
            built
        }
        "import-sort" => {
            let mut file = modified_file("src/app.ts");
            file.import_only = Some(ImportChange {
                bindings_equal: true,
                side_effect_order_changed: false,
            });
            case(vec![], vec![file])
        }
        "constant-in-auth" => {
            let mut constant = changed(
                "src/auth/limits.ts",
                "MAX_ATTEMPTS",
                NodeKind::Constant,
                body_change(),
            );
            constant
                .facts
                .insert(facts::LITERAL_INITIALIZER_ONLY.to_owned());
            let mut built = case(
                vec![constant.clone()],
                vec![modified_file("src/auth/limits.ts")],
            );
            // `canRetry` in `src/auth/guard.ts` reads it in an authorization decision.
            built.context.sensitive_constants.insert(constant.key());
            built
        }
        "snapshot-update" => case(
            vec![],
            vec![FileInput::new(
                path("src/__snapshots__/greet.spec.ts.snap"),
                FileChangeStatus::Modified,
            )],
        ),
        other => panic!("no change set for fixture {other}"),
    }
}

fn actual(name: &str) -> Value {
    let case = build(name);
    let report = detect_low_risk(&case.changes, &case.context);
    let mut symbols: Vec<Value> = case
        .changes
        .symbols
        .iter()
        .zip(&case.names)
        .map(|(symbol, (file, symbol_name))| {
            json!({
                "file": file,
                "name": symbol_name,
                "verdict": serde_json::to_value(report.per_symbol[&symbol.key()]).unwrap(),
            })
        })
        .collect();
    symbols.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_owned());
    let files: Vec<Value> = report
        .per_file
        .iter()
        .filter(|(file, verdict)| {
            // Files that only carry symbols are described by their symbols.
            verdict.kind.is_some()
                && !case
                    .changes
                    .symbols
                    .iter()
                    .any(|s| s.path() == *file)
        })
        .map(|(file, verdict)| {
            json!({"path": file.as_str(), "verdict": serde_json::to_value(verdict).unwrap()})
        })
        .collect();
    json!({"scenario": name, "symbols": symbols, "files": files})
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/pull-requests")
        .join(name)
}

const SCENARIOS: [&str; 6] = [
    "format-only",
    "pure-rename",
    "exported-rename",
    "import-sort",
    "constant-in-auth",
    "snapshot-update",
];

#[test]
fn low_risk_fixtures_match_their_expectations() {
    for name in SCENARIOS {
        let dir = fixture(name);
        for required in ["scenario.yaml", "patch.diff", "base/package.json"] {
            assert!(dir.join(required).is_file(), "{name}: missing {required}");
        }
        let expected: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("expected/low-risk.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(actual(name), expected, "fixture {name}");
    }
}

#[test]
fn only_pure_changes_without_overrides_are_low_risk() {
    let low = |name: &str| -> bool {
        let case = build(name);
        let report = detect_low_risk(&case.changes, &case.context);
        report.per_symbol.values().any(|v| v.is_low_risk())
            || report.per_file.values().any(|v| v.is_low_risk())
    };
    assert!(low("format-only"));
    assert!(low("pure-rename"));
    assert!(low("import-sort"));
    assert!(low("snapshot-update"));
    // Overridden by the public contract and by a constant read in authorization code.
    assert!(!low("exported-rename"));
    assert!(!low("constant-in-auth"));
}
