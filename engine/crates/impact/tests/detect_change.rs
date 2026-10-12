//! RISK-003: class-driven and graph-driven signals.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use codegraph::NodeKind;
use impact::input::{facts, tags, CallInput, ChangeClass, ChangeSet, ClassInput, SymbolInput};
use impact::risk::score::{assess, ScoreInput};
use impact::risk::{
    detect_all, detect_change_signals, detect_graph_signals, PathRule, RiskCategory, RiskLevel,
    RiskPolicy, RiskSignal,
};
use support::*;

fn symbol_with(classes: Vec<ClassInput>) -> SymbolInput {
    let mut symbol = changed("src/a.ts", "AService.run", NodeKind::Method, body_change());
    symbol.classes = classes;
    symbol
}

fn set_of(symbols: Vec<SymbolInput>) -> ChangeSet {
    ChangeSet {
        symbols,
        ..ChangeSet::default()
    }
}

fn change_signals(symbol: SymbolInput) -> Vec<RiskSignal> {
    detect_change_signals(&set_of(vec![symbol]), &RiskPolicy::default())
}

fn find<'a>(signals: &'a [RiskSignal], rule: &str) -> &'a RiskSignal {
    signals
        .iter()
        .find(|s| s.rule_id == rule)
        .unwrap_or_else(|| panic!("no {rule} signal in {:?}", rule_ids(signals)))
}

fn rule_ids(signals: &[RiskSignal]) -> Vec<&str> {
    signals.iter().map(|s| s.rule_id.as_str()).collect()
}

fn src_auth_policy() -> RiskPolicy {
    RiskPolicy::default().with_paths(vec![PathRule::new(
        "src/auth/**",
        RiskLevel::Critical,
        None,
    )
    .unwrap()])
}

#[test]
fn auth_bypass_emits_authz_call_removed_and_direct_role_check_and_endpoint_reachable() {
    let scenario = auth_bypass();
    let impact = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let detection = detect_all(&scenario.change, Some(&impact), &src_auth_policy());
    assert!(!detection.graph_degraded);
    let ids = rule_ids(&detection.signals);
    for expected in [
        "authz.call_removed",
        "authz.direct_role_check",
        "endpoint.reachable",
        "path.configured",
    ] {
        assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
    }
    // Class-less change classes (call removed, condition changed) add nothing of their own.
    assert!(!ids.contains(&"validation.removed"));

    let scored = assess(&ScoreInput {
        signals: detection.signals,
        ..ScoreInput::default()
    });
    assert_eq!(scored.level, RiskLevel::Critical);
    assert!((scored.score - 0.993).abs() < 0.001, "{}", scored.score);
    assert_eq!(scored.signals[0].rule_id, "authz.call_removed");
}

#[test]
fn degraded_without_impact_still_fires_class_rules() {
    let scenario = auth_bypass();
    let detection = detect_all(&scenario.change, None, &RiskPolicy::default());
    assert!(detection.graph_degraded);
    let ids = rule_ids(&detection.signals);
    assert!(ids.contains(&"authz.call_removed"));
    assert!(!ids.contains(&"endpoint.reachable"));
}

#[test]
fn guard_removed_floor_high() {
    let signals = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::AuthorizationChanged,
    )
    .tag(tags::GUARD_REMOVED)]));
    let signal = find(&signals, "authz.guard_removed");
    assert_eq!(signal.floor_level, Some(RiskLevel::High));
    assert_eq!(signal.weight, 0.85);
    let added = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::AuthorizationChanged,
    )
    .tag(tags::GUARD_ADDED)]));
    assert_eq!(find(&added, "authz.guard_added").weight, 0.4);
}

#[test]
fn untagged_authorization_change_is_logic_changed() {
    let signals = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::AuthorizationChanged,
    )]));
    assert_eq!(rule_ids(&signals), vec!["authz.logic_changed"]);
    assert_eq!(signals[0].category, RiskCategory::Authorization);
}

#[test]
fn validation_removed_signal() {
    let signals = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::ValidationRemoved,
    )
    .with_confidence(0.8)]));
    let signal = find(&signals, "validation.removed");
    assert_eq!(signal.category, RiskCategory::Validation);
    assert_eq!(signal.weight, 0.65);
    assert_eq!(signal.confidence, 0.8);
}

#[test]
fn write_in_tx_vs_outside_category() {
    let inside = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::DatabaseWriteChanged,
    )
    .tag(tags::INSIDE_TRANSACTION)]));
    let signal = find(&inside, "db.write_changed_in_tx");
    assert_eq!(signal.category, RiskCategory::TransactionBoundaries);
    assert_eq!(signal.weight, 0.45);
    let outside = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::DatabaseWriteChanged,
    )]));
    let signal = find(&outside, "db.write_changed");
    assert_eq!(signal.category, RiskCategory::SchemaChanges);
    assert_eq!(signal.weight, 0.4);
    let boundary = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::TransactionBoundaryChanged,
    )]));
    assert_eq!(find(&boundary, "tx.boundary_changed").weight, 0.6);
}

#[test]
fn breaking_vs_additive_api_weight() {
    let breaking = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::ApiContractChanged,
    )
    .tag(tags::BREAKING)]));
    assert_eq!(find(&breaking, "api.breaking_change").weight, 0.6);
    let additive = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::ApiContractChanged,
    )
    .tag(tags::ADDITIVE)]));
    assert_eq!(find(&additive, "api.additive_change").weight, 0.35);
    // Untagged contract changes are treated as breaking.
    let untagged = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::ApiContractChanged,
    )]));
    assert_eq!(rule_ids(&untagged), vec!["api.breaking_change"]);
}

#[test]
fn floating_promise_concurrency_signal() {
    let floating = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::AsyncBehaviorChanged,
    )
    .tag(tags::FLOATING_PROMISE)]));
    let signal = find(&floating, "async.floating_promise");
    assert_eq!(signal.category, RiskCategory::Concurrency);
    assert_eq!(signal.weight, 0.5);
    let await_removed = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::AsyncBehaviorChanged,
    )
    .tag(tags::AWAIT_REMOVED_ON_DB_WRITE)]));
    assert_eq!(rule_ids(&await_removed), vec!["async.floating_promise"]);
    let other = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::AsyncBehaviorChanged,
    )]));
    assert!(other.is_empty());
}

#[test]
fn catch_swallow_no_risk_signal_documented() {
    let signals = change_signals(symbol_with(vec![ClassInput::new(
        ChangeClass::ExceptionHandlingChanged,
    )
    .tag(tags::SWALLOWED)]));
    assert!(signals.is_empty());
    let doc = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/reviewers/risk-rules.md"),
    )
    .unwrap();
    assert!(doc.contains("produces **no risk-score signal**"));
}

#[test]
fn configured_authorization_symbol_counts_as_call_removed() {
    let mut symbol = symbol_with(Vec::new());
    symbol.removed_calls = vec![CallInput::unresolved("this.guard.canActivate")];
    let policy = RiskPolicy::default().with_authorization_symbols(vec!["canActivate".to_owned()]);
    let signals = detect_change_signals(&set_of(vec![symbol.clone()]), &policy);
    assert_eq!(rule_ids(&signals), vec!["authz.call_removed"]);
    assert!(detect_change_signals(&set_of(vec![symbol]), &RiskPolicy::default()).is_empty());
}

#[test]
fn cosmetic_and_test_symbols_emit_nothing() {
    let mut cosmetic = symbol_with(vec![ClassInput::new(ChangeClass::ValidationRemoved)]);
    cosmetic.cosmetic = true;
    let mut test = symbol_with(vec![ClassInput::new(ChangeClass::ValidationRemoved)]);
    test.test = true;
    assert!(change_signals(cosmetic).is_empty());
    assert!(change_signals(test).is_empty());
}

fn graph_signals(symbol: SymbolInput) -> Vec<RiskSignal> {
    detect_graph_signals(&set_of(vec![symbol]), None, &RiskPolicy::default()).signals
}

#[test]
fn queue_producer_change() {
    let mut producer = changed(
        "src/q.ts",
        "Notifier.enqueue",
        NodeKind::QueueProducer,
        body_change(),
    );
    producer.classes = Vec::new();
    let signals = graph_signals(producer);
    let signal = find(&signals, "queue.produce_changed");
    assert_eq!(signal.category, RiskCategory::Queues);
    let mut handler = changed(
        "src/q.ts",
        "Worker.process",
        NodeKind::JobHandler,
        body_change(),
    );
    handler.classes = Vec::new();
    let signals = graph_signals(handler);
    assert_eq!(
        find(&signals, "queue.consume_changed").category,
        RiskCategory::BackgroundJobs
    );
}

#[test]
fn axios_call_added_external() {
    let mut symbol = symbol_with(Vec::new());
    symbol.added_calls = vec![CallInput::to("pkg:npm/axios", "axios.get")];
    let signals = graph_signals(symbol);
    let signal = find(&signals, "external.call_added");
    assert_eq!(signal.category, RiskCategory::ExternalNetworkCalls);
    assert_eq!(signal.weight, 0.4);
    // A removed call is not an added one.
    let mut removed = symbol_with(Vec::new());
    removed.removed_calls = vec![CallInput::to("pkg:npm/axios", "axios.get")];
    assert!(graph_signals(removed).is_empty());
}

#[test]
fn jwt_verify_removed_authentication_floor() {
    let mut symbol = symbol_with(Vec::new());
    symbol.removed_calls = vec![CallInput::to("pkg:npm/jsonwebtoken", "jwt.verify")];
    let signals = graph_signals(symbol);
    let signal = find(&signals, "auth.jwt_verify_removed");
    assert_eq!(signal.category, RiskCategory::Authentication);
    assert_eq!(signal.floor_level, Some(RiskLevel::High));
    assert_eq!(
        find(&signals, "crypto.changed").category,
        RiskCategory::Cryptography
    );
}

#[test]
fn filesystem_serialization_payments_and_concurrency() {
    let mut fs_symbol = symbol_with(Vec::new());
    fs_symbol.added_calls = vec![CallInput::to("pkg:node/fs/promises", "fs.readFile")];
    assert_eq!(
        find(&graph_signals(fs_symbol), "fs.access_changed").category,
        RiskCategory::FilesystemAccess
    );

    let mut parse = symbol_with(Vec::new());
    parse.facts.insert(facts::JSON_PARSE_EXTERNAL.to_owned());
    assert_eq!(
        find(&graph_signals(parse), "serialization.changed").category,
        RiskCategory::Serialization
    );

    let mut authn = symbol_with(Vec::new());
    authn.facts.insert(facts::AUTHN_STRATEGY.to_owned());
    assert_eq!(
        find(&graph_signals(authn), "auth.authn_changed").weight,
        0.7
    );

    let mut pay = symbol_with(Vec::new());
    pay.added_calls = vec![CallInput::to("pkg:npm/stripe", "stripe.charges.create")];
    assert_eq!(
        find(&graph_signals(pay), "payments.changed").category,
        RiskCategory::Payments
    );
    let in_payments = changed(
        "src/payments/charge.ts",
        "Charge.run",
        NodeKind::Method,
        body_change(),
    );
    assert_eq!(
        find(&graph_signals(in_payments), "payments.changed").weight,
        0.65
    );

    let mut shared = symbol_with(Vec::new());
    shared.facts.insert(facts::SHARED_STATE_WRITE.to_owned());
    assert_eq!(
        find(&graph_signals(shared), "concurrency.shared_state").category,
        RiskCategory::Concurrency
    );
}

#[test]
fn report_service_decoy_no_signal() {
    // ReportService.generate also calls PermissionService.check but is not part of the change.
    let scenario = auth_bypass();
    let impact = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let detection = detect_all(&scenario.change, Some(&impact), &RiskPolicy::default());
    for signal in &detection.signals {
        let subject = format!("{:?}", signal.subject);
        let evidence = format!("{:?}", signal.evidence);
        assert!(!subject.contains("ReportService"), "{}", signal.rule_id);
        assert!(!evidence.contains("ReportService"), "{}", signal.rule_id);
    }
}
