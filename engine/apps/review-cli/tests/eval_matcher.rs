#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! EVAL-002: the deterministic matcher.

use proptest::prelude::*;
use review_cli::eval::case::Case;
use review_cli::eval::matcher::{
    match_case, ExpectationClass, FindingClass, Lineage, ProducedFinding, Stage,
};
use review_core::finding::Severity;
use serde::Deserialize;

const SYM: &str = "ts:src/ledger/ledger.service#LedgerService.close/method";
const OTHER: &str = "ts:src/ledger/ledger.service#LedgerService.import/method";

fn case(expected: &str, forbidden: &str, optional: &str) -> Case {
    let yaml = format!(
        "id: corr-900-test\nversion: 1\ntitle: t\nclasses: [correctness_regression]\nlanguage: typescript\nframeworks: []\npr: {{title: t, description: d, base_branch: main}}\nrequired_reviewers: [correctness]\nexpected_findings: {expected}\nforbidden_findings: {forbidden}\noptional_findings: {optional}\nexpect_no_published_findings: false\nsynthetic_fixtures: true\n"
    );
    serde_yaml::from_str(&yaml).unwrap()
}

fn finding(id: &str, stage: Stage, category: &str, symbol: Option<&str>, lines: (u32, u32)) -> ProducedFinding {
    ProducedFinding {
        id: id.into(),
        stage,
        category: category.into(),
        severity: Severity::High,
        primary_symbol: symbol.map(str::to_owned),
        affected_symbols: symbol.map(|s| vec![s.to_owned()]).unwrap_or_default(),
        path: "src/ledger/ledger.service.ts".into(),
        start_line: lines.0,
        end_line: lines.1,
        title: "promise not awaited".into(),
        claim: "the call is unawaited".into(),
        description: String::new(),
    }
}

fn class_of(r: &review_cli::eval::matcher::MatchResult, id: &str) -> FindingClass {
    r.findings.iter().find(|f| f.finding_id == id).unwrap().class
}

fn exp_of(r: &review_cli::eval::matcher::MatchResult, id: &str) -> ExpectationClass {
    r.expectations.iter().find(|e| e.id == id).unwrap().class
}

#[test]
fn symbol_match_beats_location_match() {
    let c = case(
        &format!("[{{id: E1, categories: [correctness], symbol: \"{SYM}\", location: {{path: src/ledger/ledger.service.ts, line: 14, window: 2}}}}]"),
        "[]",
        "[]",
    );
    // f-loc matches by location only; f-sym by symbol (and not location).
    let fs = vec![
        finding("a-loc", Stage::Published, "correctness", Some(OTHER), (14, 14)),
        finding("b-sym", Stage::Published, "correctness", Some(SYM), (40, 41)),
    ];
    let r = match_case(&c, &fs, &Lineage::new());
    assert_eq!(class_of(&r, "b-sym"), FindingClass::Tp);
    assert_eq!(class_of(&r, "a-loc"), FindingClass::Duplicate);
}

#[test]
fn location_window_inclusive() {
    let c = case(
        "[{id: E1, categories: [correctness], location: {path: src/ledger/ledger.service.ts, line: 14, window: 2}}]",
        "[]",
        "[]",
    );
    let edge = match_case(&c, &[finding("f", Stage::Published, "correctness", None, (16, 20))], &Lineage::new());
    assert_eq!(exp_of(&edge, "E1"), ExpectationClass::Hit);
    let outside = match_case(&c, &[finding("f", Stage::Published, "correctness", None, (17, 20))], &Lineage::new());
    assert_eq!(exp_of(&outside, "E1"), ExpectationClass::Miss);
}

#[test]
fn category_any_matches_all() {
    let c = case("[]", &format!("[{{id: F1, categories: [any], symbol: \"{SYM}\", reason: r}}]"), "[]");
    for cat in ["performance", "maintainability", "security"] {
        let r = match_case(&c, &[finding("f", Stage::Published, cat, Some(SYM), (1, 1))], &Lineage::new());
        assert_eq!(class_of(&r, "f"), FindingClass::FpForbidden, "{cat}");
    }
}

#[test]
fn compat_security_correctness() {
    let c = case(&format!("[{{id: E1, categories: [security], symbol: \"{SYM}\"}}]"), "[]", "[]");
    let r = match_case(&c, &[finding("f", Stage::Published, "correctness", Some(SYM), (1, 1))], &Lineage::new());
    assert_eq!(exp_of(&r, "E1"), ExpectationClass::Hit);
    let r = match_case(&c, &[finding("f", Stage::Published, "performance", Some(SYM), (1, 1))], &Lineage::new());
    assert_eq!(exp_of(&r, "E1"), ExpectationClass::Miss);
}

#[test]
fn duplicate_of_matched_expected_reported() {
    let c = case(&format!("[{{id: E1, categories: [correctness], symbol: \"{SYM}\"}}]"), "[]", "[]");
    let fs = vec![
        finding("f1", Stage::Published, "correctness", Some(SYM), (1, 1)),
        finding("f2", Stage::Published, "correctness", Some(SYM), (2, 2)),
    ];
    let r = match_case(&c, &fs, &Lineage::new());
    assert_eq!(class_of(&r, "f1"), FindingClass::Tp);
    assert_eq!(class_of(&r, "f2"), FindingClass::Duplicate);
}

#[test]
fn forbidden_published_is_fp() {
    let c = case("[]", &format!("[{{id: F1, categories: [any], symbol: \"{SYM}\", reason: r}}]"), "[]");
    let r = match_case(&c, &[finding("f", Stage::Published, "correctness", Some(SYM), (1, 1))], &Lineage::new());
    assert_eq!(class_of(&r, "f"), FindingClass::FpForbidden);
    assert_eq!(exp_of(&r, "F1"), ExpectationClass::Triggered);
}

#[test]
fn forbidden_suppressed_is_not_fp() {
    let c = case("[]", &format!("[{{id: F1, categories: [any], symbol: \"{SYM}\", reason: r}}]"), "[]");
    let f = finding("f", Stage::Suppressed("suppressed_low_confidence".into()), "correctness", Some(SYM), (1, 1));
    let r = match_case(&c, &[f], &Lineage::new());
    assert_eq!(exp_of(&r, "F1"), ExpectationClass::Clear);
}

#[test]
fn optional_is_neutral() {
    let c = case("[]", "[]", &format!("[{{id: O1, categories: [correctness], symbol: \"{SYM}\"}}]"));
    let r = match_case(&c, &[finding("f", Stage::Published, "correctness", Some(SYM), (1, 1))], &Lineage::new());
    assert_eq!(class_of(&r, "f"), FindingClass::Optional);
    assert_eq!(exp_of(&r, "O1"), ExpectationClass::OptionalHit);
}

#[test]
fn internal_only_hit_counts_only_when_must_publish_false() {
    let c = case(
        &format!("[{{id: E1, categories: [correctness], symbol: \"{SYM}\", must_publish: false}}, {{id: E2, categories: [correctness], symbol: \"{OTHER}\"}}]"),
        "[]",
        "[]",
    );
    let fs = vec![
        finding("f1", Stage::Internal, "correctness", Some(SYM), (1, 1)),
        finding("f2", Stage::Internal, "correctness", Some(OTHER), (1, 1)),
    ];
    let r = match_case(&c, &fs, &Lineage::new());
    assert_eq!(exp_of(&r, "E1"), ExpectationClass::HitInternalOnly);
    assert_eq!(exp_of(&r, "E2"), ExpectationClass::HitInternalOnly);
    let e1 = r.expectations.iter().find(|e| e.id == "E1").unwrap();
    let e2 = r.expectations.iter().find(|e| e.id == "E2").unwrap();
    assert!(!e1.must_publish && e2.must_publish);
}

#[test]
fn renamed_symbol_matches_via_lineage() {
    let renamed = "ts:src/ledger/ledger.service#LedgerService.closeAccount/method";
    let c = case(&format!("[{{id: E1, categories: [correctness], symbol: \"{SYM}\"}}]"), "[]", "[]");
    let lineage = Lineage::from([(SYM.to_owned(), renamed.to_owned())]);
    let r = match_case(&c, &[finding("f", Stage::Published, "correctness", Some(renamed), (1, 1))], &lineage);
    assert_eq!(exp_of(&r, "E1"), ExpectationClass::Hit);
    let r = match_case(&c, &[finding("f", Stage::Published, "correctness", Some(renamed), (1, 1))], &Lineage::new());
    assert_eq!(exp_of(&r, "E1"), ExpectationClass::Miss);
}

#[test]
fn underseverity_is_reported() {
    let c = case(&format!("[{{id: E1, categories: [correctness], symbol: \"{SYM}\", severity_at_least: critical}}]"), "[]", "[]");
    let r = match_case(&c, &[finding("f", Stage::Published, "correctness", Some(SYM), (1, 1))], &Lineage::new());
    assert_eq!(exp_of(&r, "E1"), ExpectationClass::HitUnderseverity);
}

proptest! {
    #[test]
    fn assignment_is_deterministic_under_permutation(seed in 0u64..1000) {
        let c = case(
            &format!("[{{id: E1, categories: [correctness], symbol: \"{SYM}\"}}, {{id: E2, categories: [correctness], location: {{path: src/ledger/ledger.service.ts, line: 30}}}}]"),
            &format!("[{{id: F1, categories: [any], symbol: \"{OTHER}\", reason: r}}]"),
            "[]",
        );
        let mut fs = vec![
            finding("f1", Stage::Published, "correctness", Some(SYM), (1, 2)),
            finding("f2", Stage::Published, "correctness", Some(SYM), (29, 31)),
            finding("f3", Stage::Published, "security", Some(OTHER), (5, 5)),
            finding("f4", Stage::Internal, "correctness", None, (30, 30)),
            finding("f5", Stage::Published, "performance", None, (90, 91)),
        ];
        let base = match_case(&c, &fs, &Lineage::new());
        // Deterministic shuffle.
        let mut s = seed;
        for i in (1..fs.len()).rev() {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let j = (s >> 33) as usize % (i + 1);
            fs.swap(i, j);
        }
        prop_assert_eq!(match_case(&c, &fs, &Lineage::new()), base);
    }
}

#[derive(Debug, Deserialize)]
struct GoldenPair {
    name: String,
    expectation: String,
    finding: GoldenFinding,
    matches: bool,
}

#[derive(Debug, Deserialize)]
struct GoldenFinding {
    category: String,
    symbol: Option<String>,
    path: String,
    lines: (u32, u32),
    text: String,
}

/// Acceptance: hand-labelled (finding, expectation) pairs agree 100% with the matcher.
#[test]
fn matcher_agrees_with_golden_labels() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/matcher_golden.yaml"),
    )
    .unwrap();
    let pairs: Vec<GoldenPair> = serde_yaml::from_str(&text).unwrap();
    assert!(pairs.len() >= 30);
    for p in pairs {
        let c = case(&format!("[{}]", p.expectation), "[]", "[]");
        let f = ProducedFinding {
            id: "f".into(),
            stage: Stage::Published,
            category: p.finding.category.clone(),
            severity: Severity::High,
            primary_symbol: p.finding.symbol.clone(),
            affected_symbols: p.finding.symbol.clone().into_iter().collect(),
            path: p.finding.path.clone(),
            start_line: p.finding.lines.0,
            end_line: p.finding.lines.1,
            title: p.finding.text.clone(),
            claim: String::new(),
            description: String::new(),
        };
        let r = match_case(&c, &[f], &Lineage::new());
        let hit = r.expectations[0].class != ExpectationClass::Miss;
        assert_eq!(hit, p.matches, "golden pair `{}`", p.name);
    }
}
