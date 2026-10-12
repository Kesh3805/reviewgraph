//! TSA-006: per-symbol syntax facts on the `ts-basic/src/facts` fixtures.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{
    compare_keys, AnalyzerConfig, AttrValue, DiagCode, FactKind, LocalId, ParsedUnit, SyntaxFact,
};
use common::{analyze, analyze_fixture, analyze_with};

fn fixture(file: &str) -> ParsedUnit {
    analyze_fixture("ts-basic", &format!("src/facts/{file}"))
}

fn local_id(unit: &ParsedUnit, qualified: &str) -> LocalId {
    unit.symbols
        .iter()
        .find(|s| s.qualified_name.join(".") == qualified)
        .unwrap_or_else(|| {
            panic!(
                "no symbol {qualified} in {:?}",
                unit.symbols
                    .iter()
                    .map(|s| s.qualified_name.join("."))
                    .collect::<Vec<_>>()
            )
        })
        .local_id
}

fn facts_of<'a>(unit: &'a ParsedUnit, qualified: &str) -> Vec<&'a SyntaxFact> {
    let id = local_id(unit, qualified);
    unit.facts
        .iter()
        .filter(|group| group.symbol == id)
        .flat_map(|group| group.facts.iter())
        .collect()
}

fn keys(facts: &[&SyntaxFact], kind: FactKind) -> Vec<String> {
    facts
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.key.clone())
        .collect()
}

fn find<'a>(facts: &[&'a SyntaxFact], kind: FactKind, prefix: &str) -> &'a SyntaxFact {
    facts
        .iter()
        .copied()
        .find(|f| f.kind == kind && f.key.starts_with(prefix))
        .unwrap_or_else(|| {
            panic!(
                "no {kind:?} fact starting with {prefix} in {:?}",
                facts.iter().map(|f| (f.kind, &f.key)).collect::<Vec<_>>()
            )
        })
}

fn flag(fact: &SyntaxFact, name: &str) -> bool {
    matches!(fact.detail.get(name), Some(AttrValue::Bool(true)))
}

fn float(fact: &SyntaxFact, name: &str) -> f64 {
    match fact.detail.get(name) {
        Some(AttrValue::Float(bits)) => f64::from_bits(bits.0),
        other => panic!("{name} is {other:?}"),
    }
}

#[test]
fn calls_and_new_keys_have_receiver_and_arity() {
    let unit = fixture("calls.ts");
    let facts = facts_of(&unit, "NotificationService.notify");
    let calls = keys(&facts, FactKind::Call);
    for expected in [
        "call:this.mailer.send/2",
        "call:this.fileStore.save/1",
        "call:console.log/1",
    ] {
        assert!(
            calls.iter().any(|k| k == expected),
            "{expected} in {calls:?}"
        );
    }
    assert_eq!(
        keys(&facts, FactKind::New),
        vec!["new:NotificationPayload/2".to_owned()]
    );
    let parse = facts_of(&unit, "NotificationService.parseInput");
    assert!(flag(
        find(&parse, FactKind::Call, "call:validateEmail/1"),
        "validation"
    ));
    assert!(flag(
        find(&parse, FactKind::Call, "call:plainToInstance/2"),
        "validation"
    ));
}

#[test]
fn condition_fact_early_exit_and_compares_null() {
    let unit = fixture("control.ts");
    let facts = facts_of(&unit, "checkAccess");
    let conditions: Vec<&&SyntaxFact> = facts
        .iter()
        .filter(|f| f.kind == FactKind::Condition)
        .collect();
    assert_eq!(conditions.len(), 2, "{conditions:?}");
    let guard = conditions[0];
    assert!(guard.key.starts_with("if:") && guard.key.len() == "if:".len() + 8);
    assert!(flag(guard, "early_exit"));
    assert!(flag(guard, "compares_null"));
    assert!(!flag(guard, "has_else"));
    let negated = conditions[1];
    assert!(flag(negated, "negated"));
    assert!(flag(negated, "has_else"));
    assert!(!flag(negated, "compares_null"));
    match negated.detail.get("idents") {
        Some(AttrValue::List(items)) => {
            assert!(
                items.contains(&AttrValue::Str("user".to_owned())),
                "{items:?}"
            )
        }
        other => panic!("idents: {other:?}"),
    }
}

#[test]
fn ternary_and_switch_conditions() {
    let unit = fixture("control.ts");
    let facts = facts_of(&unit, "label");
    let conditions = keys(&facts, FactKind::Condition);
    assert!(
        conditions.iter().any(|k| k.starts_with("ternary:")),
        "{conditions:?}"
    );
    assert!(
        conditions.iter().any(|k| k.starts_with("switch:")),
        "{conditions:?}"
    );
}

#[test]
fn loop_kinds_and_foreach() {
    let unit = fixture("control.ts");
    let facts = facts_of(&unit, "processAll");
    let loops = keys(&facts, FactKind::Loop);
    for label in ["for", "for_of", "for_in", "while", "do", "foreach"] {
        let prefix = format!("loop:{label}:");
        assert!(
            loops.iter().any(|k| k.starts_with(&prefix)),
            "{prefix} in {loops:?}"
        );
    }
    let for_of = facts
        .iter()
        .copied()
        .find(|f| f.kind == FactKind::Loop && f.key.starts_with("loop:for_of:"))
        .unwrap();
    assert!(flag(for_of, "awaits_inside"));
    let plain_for = find(&facts, FactKind::Loop, "loop:for:");
    assert!(!flag(plain_for, "awaits_inside"));
}

#[test]
fn throw_class_vs_expr() {
    let unit = fixture("errors.ts");
    assert_eq!(
        keys(&facts_of(&unit, "Importer.run"), FactKind::Throw),
        vec!["throw:ImportFailedError".to_owned()]
    );
    assert_eq!(
        keys(&facts_of(&unit, "Importer.fail"), FactKind::Throw),
        vec!["throw:expr".to_owned()]
    );
}

#[test]
fn try_catch_finally_flags_and_empty_catch() {
    let unit = fixture("errors.ts");
    let run = facts_of(&unit, "Importer.run");
    let full = find(&run, FactKind::TryCatch, "try:catch:finally");
    assert!(flag(full, "rethrows"));
    assert!(!flag(full, "empty_catch"));
    assert_eq!(
        full.detail.get("catch_param"),
        Some(&AttrValue::Str("error".to_owned()))
    );
    let quiet = facts_of(&unit, "Importer.tryQuietly");
    let empty = find(&quiet, FactKind::TryCatch, "try:catch:nofinally");
    assert!(flag(empty, "empty_catch"));
    assert!(!flag(empty, "rethrows"));
}

#[test]
fn await_and_for_await() {
    let unit = fixture("control.ts");
    let awaits = keys(&facts_of(&unit, "processAll"), FactKind::Await);
    assert!(awaits.iter().any(|k| k == "await:handle"), "{awaits:?}");
    assert!(awaits.iter().any(|k| k == "await:for_of"), "{awaits:?}");
}

#[test]
fn return_shapes() {
    let unit = fixture("control.ts");
    for (method, expected) in [
        ("none", "return:void"),
        ("nothing", "return:null"),
        ("undef", "return:undefined"),
        ("yes", "return:true"),
        ("no", "return:false"),
        ("lit", "return:lit"),
        ("ident", "return:ident"),
        ("obj", "return:obj"),
        ("call", "return:call:Math.max"),
    ] {
        let returns = keys(
            &facts_of(&unit, &format!("Shapes.{method}")),
            FactKind::Return,
        );
        assert_eq!(returns, vec![expected.to_owned()], "Shapes.{method}");
    }
    let expr = keys(&facts_of(&unit, "Shapes.expr"), FactKind::Return);
    assert!(
        expr.len() == 1 && expr[0].starts_with("return:expr:"),
        "{expr:?}"
    );
}

#[test]
fn db_write_typed_repository_save() {
    let unit = fixture("db.service.ts");
    let facts = facts_of(&unit, "UserStore.register");
    let write = find(&facts, FactKind::DbWriteLike, "dbw:save:User");
    assert!((float(write, "confidence") - 0.9).abs() < 1e-9);
    assert_eq!(
        write.detail.get("entity"),
        Some(&AttrValue::Str("User".to_owned()))
    );
    let read = facts_of(&unit, "UserStore.lookup");
    find(&read, FactKind::DbReadLike, "dbr:findOne:User");
}

#[test]
fn db_write_name_only_lower_confidence() {
    let unit = fixture("db.service.ts");
    let facts = facts_of(&unit, "UserStore.rename");
    let write = find(&facts, FactKind::DbWriteLike, "dbw:update:");
    assert!((float(write, "confidence") - 0.75).abs() < 1e-9);
}

#[test]
fn db_raw_query_classified_by_verb() {
    let unit = fixture("db.service.ts");
    let facts = facts_of(&unit, "UserStore.purge");
    let write = find(&facts, FactKind::DbWriteLike, "dbw:query:");
    assert!((float(write, "confidence") - 0.6).abs() < 1e-9);
    for fact in &facts {
        assert!(
            !fact.key.contains("DELETE"),
            "SQL text leaked: {}",
            fact.key
        );
        for value in fact.detail.values() {
            assert!(!format!("{value:?}").contains("DELETE"), "SQL text leaked");
        }
    }
}

#[test]
fn query_builder_chain_write() {
    let unit = fixture("db.service.ts");
    let facts = facts_of(&unit, "UserStore.deactivateAll");
    find(&facts, FactKind::DbWriteLike, "dbw:execute:");
    assert_eq!(
        facts
            .iter()
            .filter(|f| f.kind == FactKind::DbWriteLike)
            .count(),
        1,
        "only the terminal call writes"
    );
}

#[test]
fn non_db_save_not_flagged() {
    let unit = fixture("calls.ts");
    let facts = facts_of(&unit, "NotificationService.notify");
    find(&facts, FactKind::Call, "call:this.fileStore.save/1");
    assert!(keys(&facts, FactKind::DbWriteLike).is_empty());
}

#[test]
fn transaction_callback_marks_inner_calls() {
    let unit = fixture("transactions.ts");
    let facts = facts_of(&unit, "TransferService.transfer");
    find(
        &facts,
        FactKind::TransactionWrapper,
        "tx:this.dataSource.transaction",
    );
    assert!(flag(
        find(&facts, FactKind::Call, "call:manager.decrement/4"),
        "in_transaction"
    ));
    assert!(flag(
        find(&facts, FactKind::DbWriteLike, "dbw:increment:"),
        "in_transaction"
    ));
    assert!(!flag(
        find(&facts, FactKind::Call, "call:notifyLater/1"),
        "in_transaction"
    ));
}

#[test]
fn transactional_decorator_fact() {
    let unit = fixture("transactions.ts");
    let facts = facts_of(&unit, "TransferService.archive");
    find(&facts, FactKind::TransactionWrapper, "tx:@Transactional");
}

#[test]
fn guard_decorator_default_and_custom_list() {
    let unit = fixture("guards.ts");
    let guards =
        |unit: &ParsedUnit, symbol: &str| keys(&facts_of(unit, symbol), FactKind::GuardDecorator);
    assert!(guards(&unit, "AdminController")
        .iter()
        .any(|k| k.starts_with("guard:UseGuards:")));
    assert!(guards(&unit, "AdminController.list")
        .iter()
        .any(|k| k.starts_with("guard:Roles:")));
    assert!(guards(&unit, "AdminController.health")
        .iter()
        .any(|k| k.starts_with("guard:Public:")));
    assert!(guards(&unit, "AdminController.audit").is_empty());

    let bytes = std::fs::read(common::fixture("ts-basic").join("src/facts/guards.ts")).unwrap();
    let cfg = AnalyzerConfig {
        guard_decorator_names: Some(vec!["RequirePermission".to_owned()]),
        ..AnalyzerConfig::default()
    };
    let custom = analyze_with("src/facts/guards.ts", &bytes, &cfg);
    assert!(guards(&custom, "AdminController.audit")
        .iter()
        .any(|k| k.starts_with("guard:RequirePermission:")));
    assert!(guards(&custom, "AdminController.list").is_empty());
}

#[test]
fn process_env_read_names_only() {
    let unit = fixture("env.ts");
    let mut reads: Vec<String> = unit
        .facts
        .iter()
        .flat_map(|g| g.facts.iter())
        .filter(|f| f.kind == FactKind::ConfigRead)
        .map(|f| f.key.clone())
        .collect();
    reads.sort();
    reads.dedup();
    assert_eq!(
        reads,
        vec![
            "env:DATABASE_URL",
            "env:JWT_SECRET",
            "env:PORT",
            "env:REDIS_URL"
        ]
    );
    for fact in unit.facts.iter().flat_map(|g| g.facts.iter()) {
        assert!(!fact.key.contains("3000"), "a value leaked: {}", fact.key);
    }
}

#[test]
fn callback_facts_attributed_to_enclosing_symbol() {
    let unit = fixture("control.ts");
    let facts = facts_of(&unit, "processAll");
    find(&facts, FactKind::Call, "call:audit/1");
}

#[test]
fn module_level_facts_on_module_symbol() {
    let unit = fixture("calls.ts");
    let module = unit
        .facts
        .iter()
        .find(|g| g.symbol == LocalId(0))
        .expect("the module has facts");
    assert!(module
        .facts
        .iter()
        .any(|f| f.kind == FactKind::Call && f.key == "call:registerHandlers/0"));
}

fn key_multiset(unit: &ParsedUnit) -> Vec<(String, FactKind, String)> {
    let mut out = Vec::new();
    for group in &unit.facts {
        let owner = unit.symbols[group.symbol.0 as usize]
            .qualified_name
            .join(".");
        for fact in &group.facts {
            out.push((owner.clone(), fact.kind, fact.key.clone()));
        }
    }
    out.sort();
    out
}

#[test]
fn fact_keys_stable_under_reformat_and_comments() {
    for file in ["control.ts", "errors.ts", "db.service.ts", "calls.ts"] {
        let path = common::fixture("ts-basic").join("src/facts").join(file);
        let text = std::fs::read_to_string(path).unwrap();
        let reformatted = text
            .replace('\'', "\"")
            .replace(" {\n", " { // reviewed\n")
            .replace("(", "( ")
            .replace(", ", " ,  ");
        let base = analyze(&format!("src/facts/{file}"), &text);
        let head = analyze(&format!("src/facts/{file}"), &reformatted);
        assert_eq!(key_multiset(&base), key_multiset(&head), "{file}");
    }
}

#[test]
fn fact_cap_enforced() {
    let mut src = String::from("export function busy(): void {\n");
    for _ in 0..2_100 {
        src.push_str("  tick();\n");
    }
    src.push_str("}\nfunction tick(): void {}\n");
    let unit = analyze("src/busy.ts", &src);
    let facts = facts_of(&unit, "busy");
    assert_eq!(facts.len(), 2_000);
    assert!(unit
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::UnsupportedConstruct));
}

#[test]
fn compare_keys_multiset_delta() {
    let base = analyze(
        "src/guard.ts",
        "export function allowed(user: { admin: boolean } | null): boolean {\n  if (user === null) {\n    return false;\n  }\n  return user.admin;\n}\n",
    );
    let head = analyze(
        "src/guard.ts",
        "export function allowed(user: { admin: boolean } | null): boolean {\n  return user!.admin;\n}\n",
    );
    let delta = compare_keys(
        &facts_of(&base, "allowed")
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        &facts_of(&head, "allowed")
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
    );
    assert_eq!(delta.removed_of(FactKind::Condition).count(), 1);
    assert!(delta
        .removed_of(FactKind::Return)
        .any(|k| k == "return:false"));
    assert_eq!(delta.added_of(FactKind::Condition).count(), 0);
}

#[test]
fn syntax_facts_disabled_by_config() {
    let bytes = std::fs::read(common::fixture("ts-basic").join("src/facts/control.ts")).unwrap();
    let cfg = AnalyzerConfig {
        syntax_facts: false,
        ..AnalyzerConfig::default()
    };
    let unit = analyze_with("src/facts/control.ts", &bytes, &cfg);
    assert!(unit.facts.is_empty());
}

#[test]
fn every_fixture_unit_validates_with_facts() {
    for file in [
        "calls.ts",
        "control.ts",
        "errors.ts",
        "db.service.ts",
        "transactions.ts",
        "guards.ts",
        "env.ts",
    ] {
        let unit = fixture(file);
        assert!(!unit.facts.is_empty(), "{file} has facts");
        analysis_ir::validate(&unit).unwrap();
    }
}
