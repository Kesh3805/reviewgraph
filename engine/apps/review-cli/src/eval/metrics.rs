//! Evaluation metrics (EVAL-004). Micro-averaged over the selected cases, with per-class
//! breakdowns, Wilson 95% intervals and a baseline regression check. A metric whose
//! denominator is 0 is `null`, never 0 or 1. Rows with a `case_error` and `example-*` cases are
//! excluded (errors are counted separately).
//!
//! | Metric | Definition |
//! |---|---|
//! | `precision` | TP / (TP + FP_forbidden + FP_unexpected), published findings |
//! | `precision_strict` | same, with duplicates in the denominator |
//! | `recall` | hits(must_publish) / expected with must_publish; cases whose required reviewers are not all enabled are excluded |
//! | `recall_any_stage` | expected matched at any stage / expected |
//! | `fp_rate` | (FP_forbidden + FP_unexpected) / published |
//! | `trap_fp_rate` | cases with ≥1 published forbidden match / cases with forbidden expectations |
//! | `safe_change_fp_rate` | safe_change cases with ≥1 published / safe_change cases |
//! | `latency_p50_ms`, `latency_p95_ms` | per-case wall time, nearest-rank percentile |
//! | tokens, cost | sums and per-case means |
//! | `structured_output_success` | success / (success + failures after repair) |
//! | `verification_success` | expected matched as candidates that ended published (or internal when `must_publish: false`) / expected matched as candidates |
//! | `suppression_precision` | suppressed candidates that matched forbidden or nothing / suppressed candidates |
//! | `context_miss_rate` | missed expected whose symbol is absent from every context package / missed expected |

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::case::CaseClass;
use super::matcher::{ExpectationClass, ExpectationKind, FindingClass, Stage};
use super::result::CaseResult;

/// z for a two-sided 95% interval.
const Z: f64 = 1.96;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
}

/// A ratio with its counts and Wilson interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    pub value: Option<f64>,
    pub num: u64,
    pub den: u64,
    pub ci95: Option<Interval>,
}

impl Rate {
    pub fn new(num: u64, den: u64) -> Self {
        if den == 0 {
            return Self {
                value: None,
                num,
                den,
                ci95: None,
            };
        }
        Self {
            value: Some(num as f64 / den as f64),
            num,
            den,
            ci95: Some(wilson(num, den)),
        }
    }
}

/// Wilson score interval at 95%.
pub fn wilson(num: u64, den: u64) -> Interval {
    if den == 0 {
        return Interval { lo: 0.0, hi: 1.0 };
    }
    let n = den as f64;
    let p = num as f64 / n;
    let z2 = Z * Z;
    let denom = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denom;
    let half = Z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt() / denom;
    Interval {
        lo: (center - half).max(0.0),
        hi: (center + half).min(1.0),
    }
}

/// Nearest-rank percentile (`p` in 0..=100) of unsorted values.
pub fn percentile(values: &[u64], p: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v.get(rank.clamp(1, v.len()) - 1).copied()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub cases: u64,
    pub case_errors: u64,
    pub precision: Rate,
    pub precision_strict: Rate,
    pub recall: Rate,
    pub recall_any_stage: Rate,
    pub fp_rate: Rate,
    pub trap_fp_rate: Rate,
    pub safe_change_fp_rate: Rate,
    pub structured_output_success: Rate,
    pub verification_success: Rate,
    pub suppression_precision: Rate,
    pub context_miss_rate: Rate,
    pub latency_p50_ms: Option<u64>,
    pub latency_p95_ms: Option<u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd_micros: u64,
    pub mean_input_tokens: Option<f64>,
    pub mean_output_tokens: Option<f64>,
    pub mean_cost_usd_micros: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsReport {
    pub overall: Metrics,
    pub by_class: BTreeMap<String, Metrics>,
}

#[derive(Default)]
struct Counts {
    tp: u64,
    fp_forbidden: u64,
    fp_unexpected: u64,
    duplicates: u64,
    published: u64,
    recall_hits: u64,
    recall_den: u64,
    any_stage_hits: u64,
    any_stage_den: u64,
    trap_cases: u64,
    trap_triggered: u64,
    safe_cases: u64,
    safe_published: u64,
    so_success: u64,
    so_failures: u64,
    ver_num: u64,
    ver_den: u64,
    sup_num: u64,
    sup_den: u64,
    ctx_num: u64,
    ctx_den: u64,
}

/// Computes the metrics of `rows` (errors and examples excluded).
pub fn compute_metrics(rows: &[&CaseResult]) -> Metrics {
    let mut c = Counts::default();
    let mut latencies = Vec::new();
    let (mut input, mut output, mut cached, mut cost) = (0u64, 0u64, 0u64, 0u64);
    let mut errors = 0u64;
    let mut cases = 0u64;
    for r in rows {
        if r.is_example() {
            continue;
        }
        if r.case_error.is_some() {
            errors += 1;
            continue;
        }
        cases += 1;
        latencies.push(r.latency_ms);
        input += r.usage.input_tokens;
        output += r.usage.output_tokens;
        cached += r.usage.cached_tokens;
        cost += r.usage.cost_usd_micros;
        c.so_success += r.structured.success;
        c.so_failures += r.structured.failures_after_repair;

        let Some(m) = &r.matches else { continue };
        let mut published_any = false;
        for f in &m.findings {
            if f.stage == Stage::Published {
                published_any = true;
                c.published += 1;
                match f.class {
                    FindingClass::Tp => c.tp += 1,
                    FindingClass::FpForbidden => c.fp_forbidden += 1,
                    FindingClass::FpUnexpected => c.fp_unexpected += 1,
                    FindingClass::Duplicate => c.duplicates += 1,
                    FindingClass::Optional | FindingClass::Unmatched => {}
                }
            }
            if matches!(f.stage, Stage::Suppressed(_)) {
                c.sup_den += 1;
                if matches!(f.class, FindingClass::FpForbidden | FindingClass::Unmatched) {
                    c.sup_num += 1;
                }
            }
        }
        let context: BTreeSet<&str> = r.context_symbols.iter().map(String::as_str).collect();
        let mut has_forbidden = false;
        let mut triggered = false;
        for e in &m.expectations {
            match e.kind {
                ExpectationKind::Expected => {
                    let hit = matches!(
                        e.class,
                        ExpectationClass::Hit | ExpectationClass::HitUnderseverity
                    );
                    let any_stage = hit || e.class == ExpectationClass::HitInternalOnly;
                    if r.required_reviewers_enabled {
                        if e.must_publish {
                            c.recall_den += 1;
                            c.recall_hits += u64::from(hit);
                        }
                        c.any_stage_den += 1;
                        c.any_stage_hits += u64::from(any_stage);
                    }
                    if any_stage {
                        c.ver_den += 1;
                        let kept = hit
                            || (!e.must_publish && e.class == ExpectationClass::HitInternalOnly);
                        c.ver_num += u64::from(kept);
                    }
                    if e.class == ExpectationClass::Miss {
                        c.ctx_den += 1;
                        if e.symbol.as_deref().is_some_and(|s| !context.contains(s)) {
                            c.ctx_num += 1;
                        }
                    }
                }
                ExpectationKind::Forbidden => {
                    has_forbidden = true;
                    triggered |= e.class == ExpectationClass::Triggered;
                }
                ExpectationKind::Optional => {}
            }
        }
        if has_forbidden {
            c.trap_cases += 1;
            c.trap_triggered += u64::from(triggered);
        }
        if r.classes.contains(&CaseClass::SafeChange) {
            c.safe_cases += 1;
            c.safe_published += u64::from(published_any);
        }
    }
    let mean = |total: u64| (cases > 0).then_some(total as f64 / cases.max(1) as f64);
    Metrics {
        cases,
        case_errors: errors,
        precision: Rate::new(c.tp, c.tp + c.fp_forbidden + c.fp_unexpected),
        precision_strict: Rate::new(c.tp, c.tp + c.fp_forbidden + c.fp_unexpected + c.duplicates),
        recall: Rate::new(c.recall_hits, c.recall_den),
        recall_any_stage: Rate::new(c.any_stage_hits, c.any_stage_den),
        fp_rate: Rate::new(c.fp_forbidden + c.fp_unexpected, c.published),
        trap_fp_rate: Rate::new(c.trap_triggered, c.trap_cases),
        safe_change_fp_rate: Rate::new(c.safe_published, c.safe_cases),
        structured_output_success: Rate::new(c.so_success, c.so_success + c.so_failures),
        verification_success: Rate::new(c.ver_num, c.ver_den),
        suppression_precision: Rate::new(c.sup_num, c.sup_den),
        context_miss_rate: Rate::new(c.ctx_num, c.ctx_den),
        latency_p50_ms: percentile(&latencies, 50.0),
        latency_p95_ms: percentile(&latencies, 95.0),
        input_tokens: input,
        output_tokens: output,
        cached_tokens: cached,
        cost_usd_micros: cost,
        mean_input_tokens: mean(input),
        mean_output_tokens: mean(output),
        mean_cost_usd_micros: mean(cost),
    }
}

/// Overall metrics plus one breakdown per case class present.
pub fn compute(rows: &[CaseResult]) -> MetricsReport {
    let all: Vec<&CaseResult> = rows.iter().collect();
    let mut by_class = BTreeMap::new();
    for class in CaseClass::ALL {
        let subset: Vec<&CaseResult> = rows.iter().filter(|r| r.classes.contains(&class)).collect();
        if !subset.is_empty() {
            by_class.insert(class.as_str().to_owned(), compute_metrics(&subset));
        }
    }
    MetricsReport {
        overall: compute_metrics(&all),
        by_class,
    }
}

/// Parses results JSONL. A malformed row is an error naming its line; nothing is skipped.
pub fn parse_jsonl(text: &str) -> Result<Vec<CaseResult>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

/// One failed regression rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Regression {
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
    pub rule: String,
}

/// The regression check against a baseline (master plan §12, plus structured output):
/// precision < baseline − 0.03, fp_rate > baseline + 0.03, latency_p95 > baseline × 1.20,
/// structured_output_success < baseline − 0.05. Rules with a missing value are skipped.
pub fn regressions(current: &Metrics, baseline: &Metrics) -> Vec<Regression> {
    let mut out = Vec::new();
    let mut check =
        |name: &str, cur: Option<f64>, base: Option<f64>, rule: &str, bad: fn(f64, f64) -> bool| {
            if let (Some(c), Some(b)) = (cur, base) {
                if bad(c, b) {
                    out.push(Regression {
                        metric: name.to_owned(),
                        baseline: b,
                        current: c,
                        rule: rule.to_owned(),
                    });
                }
            }
        };
    check(
        "precision",
        current.precision.value,
        baseline.precision.value,
        "< baseline - 0.03",
        |c, b| c < b - 0.03,
    );
    check(
        "fp_rate",
        current.fp_rate.value,
        baseline.fp_rate.value,
        "> baseline + 0.03",
        |c, b| c > b + 0.03,
    );
    check(
        "latency_p95_ms",
        current.latency_p95_ms.map(|v| v as f64),
        baseline.latency_p95_ms.map(|v| v as f64),
        "> baseline * 1.20",
        |c, b| c > b * 1.20,
    );
    check(
        "structured_output_success",
        current.structured_output_success.value,
        baseline.structured_output_success.value,
        "< baseline - 0.05",
        |c, b| c < b - 0.05,
    );
    out
}
