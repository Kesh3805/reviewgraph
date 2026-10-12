#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! EVAL-004: metric definitions, intervals, percentiles and the regression check.

use review_cli::eval::case::CaseClass;
use review_cli::eval::matcher::{
    ExpectationClass, ExpectationKind, ExpectationMatch, FindingClass, FindingMatch, MatchResult,
    Stage,
};
use review_cli::eval::metrics::{
    compute, parse_jsonl, percentile, regressions, wilson, Metrics,
};
use review_cli::eval::result::{CaseResult, StructuredStats, UsageTotals};

fn f(id: &str, stage: Stage, class: FindingClass) -> FindingMatch {
    FindingMatch {
        finding_id: id.into(),
        stage,
        class,
        expectation: None,
        title: id.into(),
    }
}

fn e(id: &str, kind: ExpectationKind, class: ExpectationClass, must_publish: bool, symbol: Option<&str>) -> ExpectationMatch {
    ExpectationMatch {
        id: id.into(),
        kind,
        class,
        finding: None,
        must_publish,
        symbol: symbol.map(str::to_owned),
    }
}

fn row(id: &str, classes: Vec<CaseClass>, findings: Vec<FindingMatch>, expectations: Vec<ExpectationMatch>) -> CaseResult {
    CaseResult {
        run_id: "r".into(),
        config_name: "c".into(),
        case_id: id.into(),
        repetition: 0,
        classes,
        required_reviewers_enabled: true,
        synthetic_fixtures: true,
        case_error: None,
        replay_misses: Vec::new(),
        degraded: false,
        findings: Vec::new(),
        matches: Some(MatchResult {
            matcher_version: "1".into(),
            findings,
            expectations,
        }),
        usage: UsageTotals {
            model_calls: 1,
            input_tokens: 100,
            output_tokens: 10,
            cached_tokens: 0,
            cost_usd_micros: 5,
        },
        structured: StructuredStats {
            success: 1,
            ..StructuredStats::default()
        },
        latency_ms: 100,
        context_symbols: Vec::new(),
        calls: Vec::new(),
    }
}

use ExpectationClass as EC;
use ExpectationKind as EK;
use FindingClass as FC;

#[test]
fn precision_counts_only_published() {
    let r = row(
        "corr-001-a",
        vec![CaseClass::CorrectnessRegression],
        vec![
            f("a", Stage::Published, FC::Tp),
            f("b", Stage::Published, FC::FpUnexpected),
            f("c", Stage::Internal, FC::Unmatched),
            f("d", Stage::Suppressed("x".into()), FC::FpForbidden),
        ],
        vec![e("E1", EK::Expected, EC::Hit, true, None)],
    );
    let m = compute(&[r]).overall;
    assert_eq!((m.precision.num, m.precision.den), (1, 2));
    assert_eq!(m.fp_rate.value, Some(0.5));
}

#[test]
fn duplicates_only_in_strict() {
    let r = row(
        "corr-001-a",
        vec![CaseClass::CorrectnessRegression],
        vec![
            f("a", Stage::Published, FC::Tp),
            f("b", Stage::Published, FC::Duplicate),
        ],
        vec![e("E1", EK::Expected, EC::Hit, true, None)],
    );
    let m = compute(&[r]).overall;
    assert_eq!(m.precision.value, Some(1.0));
    assert_eq!(m.precision_strict.value, Some(0.5));
}

#[test]
fn recall_excludes_cases_missing_required_reviewers() {
    let a = row("corr-001-a", vec![], vec![], vec![e("E1", EK::Expected, EC::Hit, true, None)]);
    let mut b = row("arch-001-a", vec![], vec![], vec![e("E1", EK::Expected, EC::Miss, true, None)]);
    b.required_reviewers_enabled = false;
    let m = compute(&[a, b]).overall;
    assert_eq!((m.recall.num, m.recall.den), (1, 1));
}

#[test]
fn zero_denominator_is_null() {
    let r = row("safe-001-a", vec![CaseClass::SafeChange], vec![], vec![]);
    let m = compute(&[r]).overall;
    assert_eq!(m.precision.value, None);
    assert_eq!(m.recall.value, None);
    assert_eq!(m.trap_fp_rate.value, None);
    assert_eq!(m.safe_change_fp_rate.value, Some(0.0));
    assert!(m.precision.ci95.is_none());
}

#[test]
fn wilson_interval_known_values() {
    // 8/10 at 95%: [0.4902, 0.9433].
    let i = wilson(8, 10);
    assert!((i.lo - 0.4902).abs() < 1e-3, "{i:?}");
    assert!((i.hi - 0.9433).abs() < 1e-3, "{i:?}");
    // 0/10: [0, 0.2775].
    let z = wilson(0, 10);
    assert!(z.lo.abs() < 1e-9);
    assert!((z.hi - 0.2775).abs() < 1e-3, "{z:?}");
}

#[test]
fn percentile_nearest_rank() {
    let v = [15, 20, 35, 40, 50];
    assert_eq!(percentile(&v, 30.0), Some(20));
    assert_eq!(percentile(&v, 40.0), Some(20));
    assert_eq!(percentile(&v, 50.0), Some(35));
    assert_eq!(percentile(&v, 100.0), Some(50));
    assert_eq!(percentile(&v, 0.0), Some(15));
    assert_eq!(percentile(&[], 50.0), None);
}

fn metrics_with(precision: (u64, u64), fp: (u64, u64), p95: u64, so: (u64, u64)) -> Metrics {
    let mut rows = Vec::new();
    for i in 0..precision.0 {
        rows.push(f(&format!("t{i}"), Stage::Published, FC::Tp));
    }
    for i in 0..(precision.1 - precision.0) {
        rows.push(f(&format!("f{i}"), Stage::Published, FC::FpUnexpected));
    }
    let _ = fp;
    let mut r = row("corr-001-a", vec![], rows, vec![]);
    r.latency_ms = p95;
    r.structured = StructuredStats {
        success: so.0,
        failures_after_repair: so.1,
        ..StructuredStats::default()
    };
    compute(&[r]).overall
}

#[test]
fn regression_thresholds_trigger() {
    let base = metrics_with((9, 10), (1, 10), 1000, (10, 0));
    assert!(regressions(&base, &base).is_empty());
    // precision 0.8 < 0.9 - 0.03; fp_rate 0.2 > 0.1 + 0.03.
    let worse = metrics_with((8, 10), (2, 10), 1000, (10, 0));
    let names: Vec<String> = regressions(&worse, &base).into_iter().map(|r| r.metric).collect();
    assert!(names.contains(&"precision".to_owned()));
    assert!(names.contains(&"fp_rate".to_owned()));
    // latency 1300 > 1000 * 1.2.
    let slow = metrics_with((9, 10), (1, 10), 1300, (10, 0));
    let names: Vec<String> = regressions(&slow, &base).into_iter().map(|r| r.metric).collect();
    assert_eq!(names, ["latency_p95_ms"]);
    // structured output 0.9 < 1.0 - 0.05.
    let broken = metrics_with((9, 10), (1, 10), 1000, (9, 1));
    let names: Vec<String> = regressions(&broken, &base).into_iter().map(|r| r.metric).collect();
    assert_eq!(names, ["structured_output_success"]);
}

#[test]
fn verification_success_definition() {
    // (expectation class, must_publish) → counted in denominator, counted in numerator.
    let table = [
        (EC::Hit, true, true, true),
        (EC::HitUnderseverity, true, true, true),
        (EC::HitInternalOnly, false, true, true),
        (EC::HitInternalOnly, true, true, false),
        (EC::Miss, true, false, false),
    ];
    for (class, must_publish, in_den, in_num) in table {
        let r = row("corr-001-a", vec![], vec![], vec![e("E1", EK::Expected, class, must_publish, None)]);
        let m = compute(&[r]).overall;
        assert_eq!(m.verification_success.den, u64::from(in_den), "{class:?}");
        assert_eq!(m.verification_success.num, u64::from(in_num), "{class:?}");
    }
}

#[test]
fn context_miss_attribution() {
    let mut r = row(
        "corr-001-a",
        vec![],
        vec![],
        vec![
            e("E1", EK::Expected, EC::Miss, true, Some("ts:src/a#A.x/method")),
            e("E2", EK::Expected, EC::Miss, true, Some("ts:src/a#A.y/method")),
            e("E3", EK::Expected, EC::Hit, true, Some("ts:src/a#A.z/method")),
        ],
    );
    r.context_symbols = vec!["ts:src/a#A.y/method".into()];
    let m = compute(&[r]).overall;
    assert_eq!((m.context_miss_rate.num, m.context_miss_rate.den), (1, 2));
}

#[test]
fn trap_and_safe_change_rates() {
    let trap = row(
        "trap-001-a",
        vec![CaseClass::FpTrap],
        vec![f("a", Stage::Published, FC::FpForbidden)],
        vec![e("F1", EK::Forbidden, EC::Triggered, false, None)],
    );
    let clear = row(
        "trap-002-a",
        vec![CaseClass::FpTrap],
        vec![],
        vec![e("F1", EK::Forbidden, EC::Clear, false, None)],
    );
    let safe = row(
        "safe-001-a",
        vec![CaseClass::SafeChange],
        vec![f("a", Stage::Published, FC::FpUnexpected)],
        vec![],
    );
    let report = compute(&[trap, clear, safe]);
    assert_eq!(report.overall.trap_fp_rate.value, Some(0.5));
    assert_eq!(report.overall.safe_change_fp_rate.value, Some(1.0));
    assert_eq!(report.by_class["fp_trap"].cases, 2);
}

#[test]
fn malformed_jsonl_names_the_line() {
    let good = serde_json::to_string(&row("corr-001-a", vec![], vec![], vec![])).unwrap();
    let text = format!("{good}\n{{not json}}\n");
    let err = parse_jsonl(&text).unwrap_err();
    assert!(err.starts_with("line 2"), "{err}");
    assert_eq!(parse_jsonl(&format!("{good}\n\n{good}\n")).unwrap().len(), 2);
}

#[test]
fn examples_and_errors_are_excluded() {
    let ex = row("example-000-a", vec![], vec![f("a", Stage::Published, FC::FpUnexpected)], vec![]);
    let mut err = row("corr-001-a", vec![], vec![f("a", Stage::Published, FC::FpUnexpected)], vec![]);
    err.case_error = Some("replay_miss".into());
    let m = compute(&[ex, err]).overall;
    assert_eq!(m.cases, 0);
    assert_eq!(m.case_errors, 1);
    assert_eq!(m.fp_rate.value, None);
}
