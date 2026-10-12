//! Evaluation metrics over per-case result rows (EVAL-004). Pure computation.
//!
//! All metrics are micro-averaged over the selected cases. A metric whose denominator is zero is
//! `None` (serialised as `null`), never 0 or 1.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::case::CaseClass;
use super::matcher::{ExpectationClass, FindingClass};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpRow {
    pub id: String,
    pub class: ExpectationClass,
    pub must_publish: bool,
    /// The expected finding appeared as a candidate at some stage.
    pub candidate: bool,
    /// The expected symbol was present in some ContextPackage of the run.
    pub symbol_in_context: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingRow {
    pub class: FindingClass,
    pub published: bool,
    pub suppressed: bool,
}

/// One JSONL line of `results.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseRow {
    pub case_id: String,
    #[serde(default)]
    pub repetition: u32,
    pub classes: Vec<CaseClass>,
    /// All `required_reviewers` of the case were enabled in the run.
    pub reviewers_enabled: bool,
    pub has_forbidden: bool,
    #[serde(default)]
    pub case_error: Option<String>,
    pub latency_ms: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub cost_usd_micros: u64,
    #[serde(default)]
    pub structured_ok: u64,
    #[serde(default)]
    pub structured_fail: u64,
    #[serde(default)]
    pub expected: Vec<ExpRow>,
    #[serde(default)]
    pub findings: Vec<FindingRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub cases: usize,
    pub precision: Option<f64>,
    pub precision_ci: Option<Interval>,
    pub precision_strict: Option<f64>,
    pub recall: Option<f64>,
    pub recall_ci: Option<Interval>,
    pub recall_any_stage: Option<f64>,
    pub fp_rate: Option<f64>,
    pub fp_rate_ci: Option<Interval>,
    pub trap_fp_rate: Option<f64>,
    pub safe_change_fp_rate: Option<f64>,
    pub latency_p50_ms: Option<u64>,
    pub latency_p95_ms: Option<u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd_micros: u64,
    pub structured_output_success: Option<f64>,
    pub verification_success: Option<f64>,
    pub suppression_precision: Option<f64>,
    pub context_miss_rate: Option<f64>,
}

/// Parses JSONL; a malformed row is an error naming the 1-based line number.
pub fn parse_results(jsonl: &str) -> Result<Vec<CaseRow>> {
    let mut rows = Vec::new();
    for (i, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CaseRow>(line) {
            Ok(r) => rows.push(r),
            Err(e) => bail!("results line {}: {e}", i + 1),
        }
    }
    Ok(rows)
}

fn ratio(num: usize, den: usize) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64)
}

/// Wilson 95% interval; `None` when `n == 0`.
pub fn wilson(successes: usize, n: usize) -> Option<Interval> {
    if n == 0 {
        return None;
    }
    let z = 1.959_963_984_540_054_f64;
    let n = n as f64;
    let p = successes as f64 / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt() / denom;
    Some(Interval {
        lo: (centre - half).max(0.0),
        hi: (centre + half).min(1.0),
    })
}

/// Nearest-rank percentile (`p` in 0..=100) of an unsorted sample.
pub fn percentile(values: &[u64], p: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    let rank = ((p / 100.0) * v.len() as f64).ceil().max(1.0) as usize;
    v.get(rank - 1).copied()
}

fn is_published_hit(c: ExpectationClass) -> bool {
    matches!(
        c,
        ExpectationClass::Hit | ExpectationClass::HitUnderseverity
    )
}

/// Computes metrics over the rows. Rows with a `case_error` and `example-` cases are excluded.
pub fn compute(rows: &[CaseRow]) -> Metrics {
    let rows: Vec<&CaseRow> = rows
        .iter()
        .filter(|r| r.case_error.is_none() && !r.case_id.starts_with("example-"))
        .collect();
    let mut m = Metrics {
        cases: rows.len(),
        ..Metrics::default()
    };

    let (mut tp, mut fp_forbidden, mut fp_unexpected, mut dup, mut published) = (0, 0, 0, 0, 0);
    let (mut sup_total, mut sup_correct) = (0, 0);
    for f in rows.iter().flat_map(|r| &r.findings) {
        if f.published {
            published += 1;
            match f.class {
                FindingClass::Tp => tp += 1,
                FindingClass::FpForbidden => fp_forbidden += 1,
                FindingClass::FpUnexpected => fp_unexpected += 1,
                FindingClass::Duplicate => dup += 1,
                FindingClass::Optional | FindingClass::Ignored => {}
            }
        }
        if f.suppressed {
            sup_total += 1;
            if f.class != FindingClass::Tp {
                sup_correct += 1;
            }
        }
    }
    let fp = fp_forbidden + fp_unexpected;
    m.precision = ratio(tp, tp + fp);
    m.precision_ci = wilson(tp, tp + fp);
    m.precision_strict = ratio(tp, tp + fp + dup);
    m.fp_rate = ratio(fp, published);
    m.fp_rate_ci = wilson(fp, published);
    m.suppression_precision = ratio(sup_correct, sup_total);

    // Recall counts only cases whose required reviewers were all enabled.
    let enabled: Vec<&&CaseRow> = rows.iter().filter(|r| r.reviewers_enabled).collect();
    let must: Vec<&ExpRow> = enabled
        .iter()
        .flat_map(|r| &r.expected)
        .filter(|e| e.must_publish)
        .collect();
    let hits = must.iter().filter(|e| is_published_hit(e.class)).count();
    m.recall = ratio(hits, must.len());
    m.recall_ci = wilson(hits, must.len());
    let all_expected: Vec<&ExpRow> = enabled.iter().flat_map(|r| &r.expected).collect();
    let any_stage = all_expected
        .iter()
        .filter(|e| e.class != ExpectationClass::Miss)
        .count();
    m.recall_any_stage = ratio(any_stage, all_expected.len());

    let trap_cases = rows.iter().filter(|r| r.has_forbidden).count();
    let trap_fps = rows
        .iter()
        .filter(|r| {
            r.findings
                .iter()
                .any(|f| f.class == FindingClass::FpForbidden)
        })
        .count();
    m.trap_fp_rate = ratio(trap_fps, trap_cases);

    let safe: Vec<&&CaseRow> = rows
        .iter()
        .filter(|r| r.classes.contains(&CaseClass::SafeChange))
        .collect();
    let safe_fp = safe
        .iter()
        .filter(|r| r.findings.iter().any(|f| f.published))
        .count();
    m.safe_change_fp_rate = ratio(safe_fp, safe.len());

    let latencies: Vec<u64> = rows.iter().map(|r| r.latency_ms).collect();
    m.latency_p50_ms = percentile(&latencies, 50.0);
    m.latency_p95_ms = percentile(&latencies, 95.0);

    m.input_tokens = rows.iter().map(|r| r.input_tokens).sum();
    m.output_tokens = rows.iter().map(|r| r.output_tokens).sum();
    m.cached_tokens = rows.iter().map(|r| r.cached_tokens).sum();
    m.cost_usd_micros = rows.iter().map(|r| r.cost_usd_micros).sum();

    let ok: u64 = rows.iter().map(|r| r.structured_ok).sum();
    let fail: u64 = rows.iter().map(|r| r.structured_fail).sum();
    m.structured_output_success = (ok + fail > 0).then(|| ok as f64 / (ok + fail) as f64);

    let candidates: Vec<&ExpRow> = rows
        .iter()
        .flat_map(|r| &r.expected)
        .filter(|e| e.candidate)
        .collect();
    let kept = candidates
        .iter()
        .filter(|e| {
            is_published_hit(e.class)
                || (e.class == ExpectationClass::HitInternalOnly && !e.must_publish)
        })
        .count();
    m.verification_success = ratio(kept, candidates.len());

    let missed: Vec<&ExpRow> = rows
        .iter()
        .flat_map(|r| &r.expected)
        .filter(|e| e.class == ExpectationClass::Miss)
        .collect();
    let ctx_missed = missed.iter().filter(|e| !e.symbol_in_context).count();
    m.context_miss_rate = ratio(ctx_missed, missed.len());
    m
}

/// Returns the failed regression checks (empty = ok). Thresholds: precision -0.03, fp_rate
/// +0.03, p95 latency x1.20, structured-output success -0.05.
pub fn regressions(current: &Metrics, baseline: &Metrics) -> Vec<String> {
    let mut out = Vec::new();
    if let (Some(c), Some(b)) = (current.precision, baseline.precision) {
        if c < b - 0.03 {
            out.push(format!("precision {c:.4} < baseline {b:.4} - 0.03"));
        }
    }
    if let (Some(c), Some(b)) = (current.fp_rate, baseline.fp_rate) {
        if c > b + 0.03 {
            out.push(format!("fp_rate {c:.4} > baseline {b:.4} + 0.03"));
        }
    }
    if let (Some(c), Some(b)) = (current.latency_p95_ms, baseline.latency_p95_ms) {
        if c as f64 > b as f64 * 1.20 {
            out.push(format!("latency_p95_ms {c} > baseline {b} x 1.20"));
        }
    }
    if let (Some(c), Some(b)) = (
        current.structured_output_success,
        baseline.structured_output_success,
    ) {
        if c < b - 0.05 {
            out.push(format!(
                "structured_output_success {c:.4} < baseline {b:.4} - 0.05"
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn row(case_id: &str, findings: Vec<FindingRow>, expected: Vec<ExpRow>) -> CaseRow {
        CaseRow {
            case_id: case_id.into(),
            repetition: 0,
            classes: vec![CaseClass::CorrectnessRegression],
            reviewers_enabled: true,
            has_forbidden: false,
            case_error: None,
            latency_ms: 100,
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
            cost_usd_micros: 7,
            structured_ok: 1,
            structured_fail: 0,
            expected,
            findings,
        }
    }

    fn fr(class: FindingClass, published: bool) -> FindingRow {
        FindingRow {
            class,
            published,
            suppressed: !published,
        }
    }

    fn er(class: ExpectationClass) -> ExpRow {
        ExpRow {
            id: "E1".into(),
            class,
            must_publish: true,
            candidate: class != ExpectationClass::Miss,
            symbol_in_context: true,
        }
    }

    #[test]
    fn precision_counts_only_published_and_duplicates_only_in_strict() {
        let r = row(
            "corr-001-a",
            vec![
                fr(FindingClass::Tp, true),
                fr(FindingClass::Duplicate, true),
                fr(FindingClass::FpUnexpected, true),
                fr(FindingClass::FpUnexpected, false),
            ],
            vec![],
        );
        let m = compute(&[r]);
        assert_eq!(m.precision, Some(0.5));
        assert!((m.precision_strict.unwrap() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn recall_excludes_cases_missing_required_reviewers() {
        let mut r = row("corr-001-a", vec![], vec![er(ExpectationClass::Miss)]);
        r.reviewers_enabled = false;
        assert_eq!(compute(&[r]).recall, None);
    }

    #[test]
    fn zero_denominator_is_null() {
        let m = compute(&[]);
        assert_eq!(m.precision, None);
        assert_eq!(m.fp_rate, None);
    }

    #[test]
    fn wilson_interval_known_values() {
        let i = wilson(5, 10).unwrap();
        assert!((i.lo - 0.2366).abs() < 1e-3, "{i:?}");
        assert!((i.hi - 0.7634).abs() < 1e-3, "{i:?}");
        assert_eq!(wilson(0, 0), None);
    }

    #[test]
    fn percentile_nearest_rank() {
        let v = [15, 20, 35, 40, 50];
        assert_eq!(percentile(&v, 50.0), Some(35));
        assert_eq!(percentile(&v, 95.0), Some(50));
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn regression_thresholds_trigger() {
        let base = Metrics {
            precision: Some(0.9),
            fp_rate: Some(0.1),
            latency_p95_ms: Some(1000),
            structured_output_success: Some(0.99),
            ..Metrics::default()
        };
        assert!(regressions(&base, &base).is_empty());
        let worse = Metrics {
            precision: Some(0.8),
            fp_rate: Some(0.2),
            latency_p95_ms: Some(1300),
            structured_output_success: Some(0.9),
            ..Metrics::default()
        };
        assert_eq!(regressions(&worse, &base).len(), 4);
    }

    #[test]
    fn verification_success_and_context_miss() {
        let rows = [row(
            "corr-001-a",
            vec![],
            vec![
                er(ExpectationClass::Hit),
                ExpRow {
                    candidate: true,
                    ..er(ExpectationClass::HitInternalOnly)
                },
                ExpRow {
                    symbol_in_context: false,
                    ..er(ExpectationClass::Miss)
                },
            ],
        )];
        let m = compute(&rows);
        assert_eq!(m.verification_success, Some(0.5));
        assert_eq!(m.context_miss_rate, Some(1.0));
    }

    #[test]
    fn malformed_row_reports_line_number() {
        let err = parse_results("\n{not json}\n").unwrap_err().to_string();
        assert!(err.contains("line 2"), "{err}");
    }
}
