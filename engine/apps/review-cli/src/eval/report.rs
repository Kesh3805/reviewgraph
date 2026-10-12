//! Evaluation reports and the `model_eval_results` table (EVAL-005).
//!
//! Rendering is deterministic: sorted tables, no wall-clock values except the run's own
//! `started_at`, so re-rendering the same run gives identical files. The Markdown is produced by
//! plain formatting code rather than a template engine, to avoid a template dependency.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::matcher::{ExpectationClass, FindingClass, Stage};
use super::metrics::{compute, regressions, Metrics, MetricsReport, Rate, Regression};
use super::result::CaseResult;

/// Written by `review eval run` next to `results.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub config_name: String,
    pub config_hash: String,
    pub corpus_version: String,
    pub corpus_hash: String,
    pub engine_git_sha: String,
    pub started_at: String,
    pub verification_version: String,
    pub gateway_mode: String,
    pub repetitions: u32,
}

/// The JSON report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub manifest: RunManifest,
    pub routing_table_hashes: Vec<String>,
    pub prompt_versions: Vec<String>,
    pub reviewer_versions: BTreeMap<String, String>,
    pub synthetic_fixtures: bool,
    pub verification_ran: bool,
    pub metrics: MetricsReport,
    pub regressions: Vec<Regression>,
    pub cases: Vec<CaseRow>,
    pub structured_failures_by_task: BTreeMap<String, u64>,
    pub context_misses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseRow {
    pub case_id: String,
    pub repetition: u32,
    pub error: Option<String>,
    pub hits: Vec<String>,
    pub misses: Vec<String>,
    pub false_positives: Vec<String>,
    pub latency_ms: u64,
}

fn fmt_rate(r: &Rate) -> String {
    match (r.value, r.ci95) {
        (Some(v), Some(ci)) => format!("{v:.3} [{:.3}, {:.3}] ({}/{})", ci.lo, ci.hi, r.num, r.den),
        _ => format!("n/a ({}/{})", r.num, r.den),
    }
}

fn opt(v: Option<u64>) -> String {
    v.map_or_else(|| "n/a".to_owned(), |v| v.to_string())
}

/// Builds the report model from the run rows.
pub fn build_report(
    manifest: &RunManifest,
    rows: &[CaseResult],
    baseline: Option<&Metrics>,
    reviewer_versions: BTreeMap<String, String>,
) -> Report {
    let metrics = compute(rows);
    let regs = baseline.map_or_else(Vec::new, |b| regressions(&metrics.overall, b));
    let mut tables = std::collections::BTreeSet::new();
    let mut prompts = std::collections::BTreeSet::new();
    let mut so_failures: BTreeMap<String, u64> = BTreeMap::new();
    let mut context_misses = Vec::new();
    let mut cases = Vec::new();
    for r in rows {
        for c in &r.calls {
            tables.insert(c.routing_table_hash.clone());
            prompts.insert(c.prompt_version.clone());
        }
        for (task, n) in &r.structured.failures_by_task {
            *so_failures.entry(task.clone()).or_default() += n;
        }
        let mut row = CaseRow {
            case_id: r.case_id.clone(),
            repetition: r.repetition,
            error: r.case_error.clone(),
            hits: Vec::new(),
            misses: Vec::new(),
            false_positives: Vec::new(),
            latency_ms: r.latency_ms,
        };
        if let Some(m) = &r.matches {
            for e in &m.expectations {
                match e.class {
                    ExpectationClass::Hit
                    | ExpectationClass::HitUnderseverity
                    | ExpectationClass::HitInternalOnly => row.hits.push(e.id.clone()),
                    ExpectationClass::Miss => {
                        row.misses.push(e.id.clone());
                        if let Some(s) = &e.symbol {
                            if !r.context_symbols.contains(s) {
                                context_misses.push(format!("{} {}: {s}", r.case_id, e.id));
                            }
                        }
                    }
                    _ => {}
                }
            }
            for f in &m.findings {
                if f.stage == Stage::Published
                    && matches!(
                        f.class,
                        FindingClass::FpForbidden | FindingClass::FpUnexpected
                    )
                {
                    row.false_positives.push(f.title.clone());
                }
            }
        }
        cases.push(row);
    }
    cases.sort_by(|a, b| {
        a.case_id
            .cmp(&b.case_id)
            .then(a.repetition.cmp(&b.repetition))
    });
    context_misses.sort();
    Report {
        manifest: manifest.clone(),
        routing_table_hashes: tables.into_iter().collect(),
        prompt_versions: prompts.into_iter().collect(),
        reviewer_versions,
        synthetic_fixtures: rows.iter().any(|r| r.synthetic_fixtures),
        verification_ran: manifest.verification_version != "none",
        metrics,
        regressions: regs,
        cases,
        structured_failures_by_task: so_failures,
        context_misses,
    }
}

fn metrics_table(out: &mut String, m: &Metrics) {
    let rows = [
        ("precision", fmt_rate(&m.precision)),
        ("precision_strict", fmt_rate(&m.precision_strict)),
        ("recall", fmt_rate(&m.recall)),
        ("recall_any_stage", fmt_rate(&m.recall_any_stage)),
        ("fp_rate", fmt_rate(&m.fp_rate)),
        ("trap_fp_rate", fmt_rate(&m.trap_fp_rate)),
        ("safe_change_fp_rate", fmt_rate(&m.safe_change_fp_rate)),
        (
            "structured_output_success",
            fmt_rate(&m.structured_output_success),
        ),
        ("verification_success", fmt_rate(&m.verification_success)),
        ("suppression_precision", fmt_rate(&m.suppression_precision)),
        ("context_miss_rate", fmt_rate(&m.context_miss_rate)),
    ];
    out.push_str("| Metric | Value [95% CI] (num/den) |\n|---|---|\n");
    for (k, v) in rows {
        let _ = writeln!(out, "| `{k}` | {v} |");
    }
}

/// Renders the Markdown report.
pub fn render_markdown(r: &Report) -> String {
    let mut out = String::new();
    let m = &r.manifest;
    let _ = writeln!(
        out,
        "# Evaluation report: {} ({})\n",
        m.config_name, m.started_at
    );
    if r.synthetic_fixtures {
        out.push_str("> **Synthetic fixtures.** At least one case used hand-authored replay fixtures. This run measures pipeline and verification behaviour, not model quality.\n\n");
    }
    if !r.verification_ran {
        out.push_str("> **Verification not run.** Every normalised reviewer candidate counts as published; rejected candidates count as suppressed.\n\n");
    }
    out.push_str("## Run\n\n| Field | Value |\n|---|---|\n");
    for (k, v) in [
        ("run id", m.run_id.clone()),
        ("config", format!("{} (`{}`)", m.config_name, m.config_hash)),
        (
            "corpus",
            format!("{} (`{}`)", m.corpus_version, m.corpus_hash),
        ),
        ("engine git sha", m.engine_git_sha.clone()),
        ("gateway mode", m.gateway_mode.clone()),
        ("routing table hash", r.routing_table_hashes.join(", ")),
        ("prompt versions", r.prompt_versions.join(", ")),
        (
            "reviewer versions",
            r.reviewer_versions
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        ("verification version", m.verification_version.clone()),
        ("repetitions", m.repetitions.to_string()),
        ("date", m.started_at.clone()),
    ] {
        let _ = writeln!(out, "| {k} | {v} |");
    }

    let o = &r.metrics.overall;
    let _ = writeln!(
        out,
        "\n## Headline metrics\n\n{} cases, {} case errors.\n",
        o.cases, o.case_errors
    );
    metrics_table(&mut out, o);

    out.push_str("\n## By class\n\n| Class | Cases | Precision | Recall | FP rate | Trap FP rate | Safe-change FP rate |\n|---|---|---|---|---|---|---|\n");
    for (class, cm) in &r.metrics.by_class {
        let v = |x: &Rate| {
            x.value
                .map_or_else(|| "n/a".to_owned(), |v| format!("{v:.3}"))
        };
        let _ = writeln!(
            out,
            "| {class} | {} | {} | {} | {} | {} | {} |",
            cm.cases,
            v(&cm.precision),
            v(&cm.recall),
            v(&cm.fp_rate),
            v(&cm.trap_fp_rate),
            v(&cm.safe_change_fp_rate)
        );
    }

    out.push_str("\n## Cases\n\n| Case | Rep | Hits | Misses | False positives | Latency ms | Error |\n|---|---|---|---|---|---|---|\n");
    for c in &r.cases {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            c.case_id,
            c.repetition,
            c.hits.join(", "),
            c.misses.join(", "),
            c.false_positives.join("; ").replace('|', "\\|"),
            c.latency_ms,
            c.error.as_deref().unwrap_or("")
        );
    }

    out.push_str("\n## Regressions vs baseline\n\n");
    if r.regressions.is_empty() {
        out.push_str("None.\n");
    } else {
        for g in &r.regressions {
            let _ = writeln!(
                out,
                "- `{}`: {:.4} vs baseline {:.4} (rule: {})",
                g.metric, g.current, g.baseline, g.rule
            );
        }
    }

    let _ = writeln!(
        out,
        "\n## Cost and latency\n\n| Field | Value |\n|---|---|\n| input tokens | {} |\n| output tokens | {} |\n| cached tokens | {} |\n| cost (micro-USD) | {} |\n| latency p50 ms | {} |\n| latency p95 ms | {} |",
        o.input_tokens,
        o.output_tokens,
        o.cached_tokens,
        o.cost_usd_micros,
        opt(o.latency_p50_ms),
        opt(o.latency_p95_ms)
    );

    out.push_str("\n## Structured-output failures by task\n\n");
    if r.structured_failures_by_task.is_empty() {
        out.push_str("None.\n");
    } else {
        for (t, n) in &r.structured_failures_by_task {
            let _ = writeln!(out, "- `{t}`: {n}");
        }
    }

    out.push_str("\n## Context misses\n\n");
    if r.context_misses.is_empty() {
        out.push_str("None.\n");
    } else {
        for c in &r.context_misses {
            let _ = writeln!(out, "- {c}");
        }
    }
    out
}

/// Writes `<dir>/<YYYY-MM-DD>-<config>.md` and `.json`. Returns both paths.
pub fn write_report(dir: &Path, r: &Report) -> anyhow::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let date: String = r.manifest.started_at.chars().take(10).collect();
    let stem = format!("{date}-{}", r.manifest.config_name);
    let md = dir.join(format!("{stem}.md"));
    let js = dir.join(format!("{stem}.json"));
    std::fs::write(&md, render_markdown(r))?;
    let mut text = serde_json::to_string_pretty(r)?;
    text.push('\n');
    std::fs::write(&js, text)?;
    Ok((md, js))
}

/// Writes `baselines/<config>.json`; refuses when the run has case errors.
pub fn update_baseline(dir: &Path, rows: &[CaseResult], r: &Report) -> anyhow::Result<PathBuf> {
    let errors: Vec<&str> = rows
        .iter()
        .filter(|r| r.case_error.is_some())
        .map(|r| r.case_id.as_str())
        .collect();
    if !errors.is_empty() {
        bail!(
            "refusing to update the baseline: case errors in {}",
            errors.join(", ")
        );
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", r.manifest.config_name));
    let mut text = serde_json::to_string_pretty(&r.metrics)?;
    text.push('\n');
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// One `model_eval_results` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvalRow {
    pub tier: String,
    pub provider: String,
    pub model: String,
    pub routing_table_hash: String,
    pub prompt_versions: Value,
    pub metrics: Value,
}

/// One row per `(tier, provider, model)` used by the run, with that tier's call stats.
pub fn eval_rows(rows: &[CaseResult], report: &Report) -> Vec<EvalRow> {
    #[derive(Default)]
    struct Agg {
        calls: u64,
        input: u64,
        output: u64,
        cost: u64,
        tables: std::collections::BTreeSet<String>,
        prompts: std::collections::BTreeSet<String>,
    }
    let mut by_key: BTreeMap<(String, String, String), Agg> = BTreeMap::new();
    for r in rows {
        for c in &r.calls {
            let a = by_key
                .entry((c.tier.clone(), c.provider.clone(), c.model.clone()))
                .or_default();
            a.calls += c.calls;
            a.input += c.input_tokens;
            a.output += c.output_tokens;
            a.cost += c.cost_usd_micros;
            a.tables.insert(c.routing_table_hash.clone());
            a.prompts.insert(c.prompt_version.clone());
        }
    }
    by_key
        .into_iter()
        .map(|((tier, provider, model), a)| EvalRow {
            routing_table_hash: a.tables.iter().cloned().collect::<Vec<_>>().join(","),
            prompt_versions: json!(a.prompts),
            metrics: json!({
                "overall": report.metrics.overall,
                "tier_calls": {"calls": a.calls, "input_tokens": a.input, "output_tokens": a.output, "cost_usd_micros": a.cost},
            }),
            tier,
            provider,
            model,
        })
        .collect()
}

fn dec(r: &Rate) -> Option<f64> {
    r.value.map(|v| (v * 10_000.0).round() / 10_000.0)
}

/// Inserts the rows; idempotent through `UNIQUE (eval_run_id, tier, provider, model)`.
/// Returns the number of rows inserted.
pub async fn persist(
    pool: &sqlx::PgPool,
    rows: &[CaseResult],
    report: &Report,
    report_path: &str,
) -> anyhow::Result<u64> {
    let m = &report.manifest;
    let o = &report.metrics.overall;
    let mut inserted = 0;
    for row in eval_rows(rows, report) {
        let res = sqlx::query(
            "INSERT INTO model_eval_results (
               id, eval_run_id, config_name, config_hash, corpus_version, corpus_hash, engine_git_sha,
               routing_table_hash, tier, provider, model, prompt_versions, reviewer_versions,
               verification_version, synthetic_fixtures, cases_total, repetitions,
               precision, recall, fp_rate, trap_fp_rate, safe_change_fp_rate,
               structured_output_success, verification_success, suppression_precision,
               latency_p50_ms, latency_p95_ms, input_tokens, output_tokens, cached_tokens,
               cost_usd_micros, metrics, report_path)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                     $18::float8::numeric(5,4),$19::float8::numeric(5,4),$20::float8::numeric(5,4),
                     $21::float8::numeric(5,4),$22::float8::numeric(5,4),$23::float8::numeric(5,4),
                     $24::float8::numeric(5,4),$25::float8::numeric(5,4),
                     $26,$27,$28,$29,$30,$31,$32,$33)
             ON CONFLICT (eval_run_id, tier, provider, model) DO NOTHING",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(m.run_id.as_str())
        .bind(m.config_name.as_str())
        .bind(m.config_hash.as_str())
        .bind(m.corpus_version.as_str())
        .bind(m.corpus_hash.as_str())
        .bind(m.engine_git_sha.as_str())
        .bind(row.routing_table_hash.as_str())
        .bind(row.tier.as_str())
        .bind(row.provider.as_str())
        .bind(row.model.as_str())
        .bind(row.prompt_versions.clone())
        .bind(json!(report.reviewer_versions))
        .bind(m.verification_version.as_str())
        .bind(report.synthetic_fixtures)
        .bind(i32::try_from(o.cases).unwrap_or(i32::MAX))
        .bind(i32::try_from(m.repetitions).unwrap_or(i32::MAX))
        .bind(dec(&o.precision))
        .bind(dec(&o.recall))
        .bind(dec(&o.fp_rate))
        .bind(dec(&o.trap_fp_rate))
        .bind(dec(&o.safe_change_fp_rate))
        .bind(dec(&o.structured_output_success))
        .bind(dec(&o.verification_success))
        .bind(dec(&o.suppression_precision))
        .bind(o.latency_p50_ms.map(|v| i32::try_from(v).unwrap_or(i32::MAX)))
        .bind(o.latency_p95_ms.map(|v| i32::try_from(v).unwrap_or(i32::MAX)))
        .bind(i64::try_from(o.input_tokens).unwrap_or(i64::MAX))
        .bind(i64::try_from(o.output_tokens).unwrap_or(i64::MAX))
        .bind(i64::try_from(o.cached_tokens).unwrap_or(i64::MAX))
        .bind(i64::try_from(o.cost_usd_micros).unwrap_or(i64::MAX))
        .bind(row.metrics.clone())
        .bind(report_path)
        .execute(pool)
        .await
        .context("inserting model_eval_results")?;
        inserted += res.rows_affected();
    }
    Ok(inserted)
}
