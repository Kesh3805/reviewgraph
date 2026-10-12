//! RISK-006 low-risk detection tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use codegraph::NodeKind;
use impact::input::{
    facts, ChangeClass, ChangeSet, ClassInput, FileInput, ImportChange, SymbolInput,
};
use impact::risk::low_risk::{detect_low_risk, LowRiskContext, LowRiskKind, OverrideReason};
use review_core::change::SymbolChange;
use review_core::ids::SymbolId;
use support::{body_change, changed, modified_file, path};

fn set(symbols: Vec<SymbolInput>, files: Vec<FileInput>) -> ChangeSet {
    ChangeSet {
        symbols,
        files,
        ..ChangeSet::default()
    }
}

#[test]
fn comment_only_low_risk_and_logic_not() {
    let mut fmt = changed(
        "src/format.ts",
        "formatName",
        NodeKind::Function,
        body_change(),
    );
    fmt.cosmetic = true;
    let mut logic = changed(
        "src/auth/a.ts",
        "authorize",
        NodeKind::Function,
        body_change(),
    );
    logic
        .classes
        .push(ClassInput::new(ChangeClass::AuthorizationChanged));
    let (fk, lk) = (fmt.key(), logic.key());
    let report = detect_low_risk(&set(vec![fmt, logic], vec![]), &LowRiskContext::default());
    assert!(report.symbol_is_low_risk(&fk));
    assert!(!report.symbol_is_low_risk(&lk));
}

#[test]
fn exported_rename_with_external_callers_overridden() {
    let change = SymbolChange::Renamed {
        from: SymbolId::from_canonical_unchecked("old".to_owned()),
        similarity: 1.0,
    };
    let mut renamed = changed("src/a.ts", "newName", NodeKind::Function, change);
    renamed.exported = true;
    let key = renamed.key();
    let mut ctx = LowRiskContext::default();
    let internal = detect_low_risk(&set(vec![renamed.clone()], vec![]), &ctx);
    assert!(internal.symbol_is_low_risk(&key));
    ctx.symbols_with_external_callers.insert(key);
    let report = detect_low_risk(&set(vec![renamed], vec![]), &ctx);
    assert!(!report.symbol_is_low_risk(&key));
    assert_eq!(
        report.per_symbol[&key].overridden_by,
        Some(OverrideReason::PublicContract)
    );
}

#[test]
fn constant_used_in_auth_overridden() {
    let mut constant = changed("src/c.ts", "MAX_ROLE", NodeKind::Variable, body_change());
    constant
        .facts
        .insert(facts::LITERAL_INITIALIZER_ONLY.to_owned());
    let key = constant.key();
    let mut ctx = LowRiskContext::default();
    let plain = detect_low_risk(&set(vec![constant.clone()], vec![]), &ctx);
    assert_eq!(
        plain.per_symbol[&key].kind,
        Some(LowRiskKind::SimpleConstant)
    );
    assert!(plain.symbol_is_low_risk(&key));
    ctx.sensitive_constants.insert(key);
    assert!(!detect_low_risk(&set(vec![constant], vec![]), &ctx).symbol_is_low_risk(&key));
}

#[test]
fn import_sort_and_side_effect_reorder() {
    let mut sorted = modified_file("src/i.ts");
    sorted.import_only = Some(ImportChange {
        bindings_equal: true,
        side_effect_order_changed: false,
    });
    let mut reordered = modified_file("src/j.ts");
    reordered.import_only = Some(ImportChange {
        bindings_equal: true,
        side_effect_order_changed: true,
    });
    let report = detect_low_risk(
        &set(vec![], vec![sorted, reordered]),
        &LowRiskContext::default(),
    );
    assert!(report.file_is_low_risk(&path("src/i.ts")));
    assert!(!report.file_is_low_risk(&path("src/j.ts")));
}

#[test]
fn snapshot_update_low_risk() {
    let file = modified_file("src/__snapshots__/a.spec.ts.snap");
    let report = detect_low_risk(&set(vec![], vec![file]), &LowRiskContext::default());
    assert!(report.file_is_low_risk(&path("src/__snapshots__/a.spec.ts.snap")));
}

#[test]
fn degraded_parse_never_low_risk_and_critical_path_overridden() {
    let mut degraded = changed("src/d.ts", "d", NodeKind::Function, body_change());
    degraded.cosmetic = true;
    degraded.parse_degraded = true;
    let mut on_path = changed("src/auth/p.ts", "p", NodeKind::Function, body_change());
    on_path.cosmetic = true;
    let (dk, pk) = (degraded.key(), on_path.key());
    let mut ctx = LowRiskContext::default();
    ctx.high_risk_path_files.insert(path("src/auth/p.ts"));
    let report = detect_low_risk(&set(vec![degraded, on_path], vec![]), &ctx);
    assert!(!report.symbol_is_low_risk(&dk));
    assert!(!report.symbol_is_low_risk(&pk));
    assert_eq!(
        report.per_symbol[&pk].overridden_by,
        Some(OverrideReason::RiskPath)
    );
}
