//! `review eval`: the model evaluation harness (EVAL-001..006). Developer tooling; the subcommand
//! is hidden from `review --help`.
//!
//! - `review eval validate [--cases <glob>]`
//! - `review eval run --config <file> [--cases <glob>|--smoke] [--out <dir>] [--baseline <file>] [--yes]`
//! - `review eval metrics --results <jsonl> [--baseline <json>]` (exit 0 ok, 2 regression, 1 error)
//! - `review eval report --results <jsonl> [--persist] [--update-baseline]`
//! - `review eval fixture new --case <id>`

pub mod build;
pub mod case;
pub mod category_compat;
pub mod fixture;
pub mod matcher;
pub mod metrics;
pub mod report;
pub mod result;
pub mod runner;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context};
use clap::Subcommand;

use case::{corpus_hash, load_corpus, EvalPaths};
use report::RunManifest;
use runner::{run_cases, EvalGatewayMode, ReviewerExecutor, RunConfig};

/// Corpus format version recorded in reports.
pub const CORPUS_VERSION: &str = "v1";

#[derive(Debug, Subcommand)]
pub enum EvalCommand {
    /// Validate benchmark cases (exit 1 on any invalid case).
    Validate {
        #[arg(long)]
        cases: Option<String>,
    },
    /// Run cases under a model configuration and write results JSONL.
    Run {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, conflicts_with = "smoke")]
        cases: Option<String>,
        /// Only the cases listed in benchmarks/quality/smoke.txt.
        #[arg(long)]
        smoke: bool,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        baseline: Option<PathBuf>,
        /// Confirm a live run whose estimated cost exceeds $5.
        #[arg(long)]
        yes: bool,
    },
    /// Compute metrics from results JSONL.
    Metrics {
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        baseline: Option<PathBuf>,
    },
    /// Render Markdown/JSON reports; optionally persist and update the baseline.
    Report {
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        baseline: Option<PathBuf>,
        #[arg(long)]
        persist: bool,
        #[arg(long)]
        update_baseline: bool,
    },
    /// Replay fixture tooling.
    Fixture {
        #[command(subcommand)]
        action: FixtureAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum FixtureAction {
    /// Write a template fixture for a case's correctness request and print its path.
    New {
        #[arg(long)]
        case: String,
    },
}

fn paths() -> anyhow::Result<EvalPaths> {
    let cwd = std::env::current_dir()?;
    EvalPaths::discover(&cwd)
}

fn git_sha(root: &Path) -> String {
    if let Ok(s) = std::env::var("RG_GIT_SHA") {
        if !s.is_empty() {
            return s;
        }
    }
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The case ids of `smoke.txt`.
pub fn smoke_ids(paths: &EvalPaths) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(paths.quality().join("smoke.txt"))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

/// Runs a command; returns the process exit code.
pub fn dispatch(cmd: EvalCommand) -> anyhow::Result<i32> {
    match cmd {
        EvalCommand::Validate { cases } => validate(cases.as_deref()),
        EvalCommand::Run {
            config,
            cases,
            smoke,
            out,
            baseline,
            yes,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(run(&config, cases.as_deref(), smoke, out, baseline, yes))
        }
        EvalCommand::Metrics { results, baseline } => metrics_cmd(&results, baseline.as_deref()),
        EvalCommand::Report {
            results,
            baseline,
            persist,
            update_baseline,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(report_cmd(
                &results,
                baseline.as_deref(),
                persist,
                update_baseline,
            ))
        }
        EvalCommand::Fixture {
            action: FixtureAction::New { case },
        } => {
            let p = paths()?;
            let (cases, errors) = load_corpus(&p, Some(&case))?;
            if let Some(e) = errors.values().next() {
                bail!("{}", e.join("\n"));
            }
            let Some(c) = cases.first() else {
                bail!("no case {case}");
            };
            let (hash, _) = fixture::expected_fixture(&p.fixtures(), c)?;
            let path = fixture::write_template(&p.fixtures(), c)?;
            println!("{hash} {}", path.display());
            Ok(0)
        }
    }
}

fn validate(filter: Option<&str>) -> anyhow::Result<i32> {
    let p = paths()?;
    let (ok, errors) = load_corpus(&p, filter)?;
    for (case, errs) in &errors {
        for e in errs {
            eprintln!("{case}: {e}");
        }
    }
    println!(
        "{} valid, {} invalid; corpus hash {}",
        ok.len(),
        errors.len(),
        corpus_hash(&p, &ok)?
    );
    Ok(i32::from(!errors.is_empty()))
}

async fn run(
    config: &Path,
    filter: Option<&str>,
    smoke: bool,
    out: Option<PathBuf>,
    baseline: Option<PathBuf>,
    yes: bool,
) -> anyhow::Result<i32> {
    let p = paths()?;
    let cfg = RunConfig::load(config)?;
    let (mut cases, errors) = load_corpus(&p, filter)?;
    if !errors.is_empty() {
        bail!(
            "invalid cases: {}",
            errors.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    let corpus = corpus_hash(&p, &cases)?;
    if smoke {
        let ids = smoke_ids(&p)?;
        cases.retain(|c| ids.contains(&c.case.id));
    }
    let gateway = match cfg.gateway_mode {
        EvalGatewayMode::Replay => runner::replay_gateway(&p.fixtures())?,
        EvalGatewayMode::Live => {
            let estimate = runner::estimate_live_usd(cases.len(), cfg.repetitions, 20_000, 3.0);
            println!("estimated live cost: ${estimate:.2}");
            runner::confirm_live(estimate, yes)?;
            live_gateway(&p, &cfg)?
        }
    };
    let started_at = chrono::Utc::now().to_rfc3339();
    let sha = git_sha(&p.root);
    let run_id = runner::run_id(&cfg.hash(), &corpus, &sha, &started_at);
    let executor = Arc::new(ReviewerExecutor::new(gateway, &cfg.reviewers)?);
    let rows = run_cases(&cases, executor, &cfg, &run_id).await;
    let out_dir = out.unwrap_or_else(|| p.root.join("engine/target/eval").join(&run_id));
    runner::write_jsonl(&out_dir.join("results.jsonl"), &rows)?;
    let manifest = RunManifest {
        run_id: run_id.clone(),
        config_name: cfg.name.clone(),
        config_hash: cfg.hash(),
        corpus_version: CORPUS_VERSION.into(),
        corpus_hash: corpus,
        engine_git_sha: sha,
        started_at,
        verification_version: cfg.verification_version.clone(),
        gateway_mode: format!("{:?}", cfg.gateway_mode).to_lowercase(),
        repetitions: cfg.repetitions,
    };
    std::fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    println!("{}", out_dir.join("results.jsonl").display());
    let misses: Vec<&str> = rows
        .iter()
        .filter(|r| r.case_error.as_deref() == Some("replay_miss"))
        .map(|r| r.case_id.as_str())
        .collect();
    if !misses.is_empty() {
        for r in rows.iter().filter(|r| !r.replay_misses.is_empty()) {
            eprintln!(
                "replay miss in {}: {}",
                r.case_id,
                r.replay_misses.join(", ")
            );
        }
        if cfg.gateway_mode == EvalGatewayMode::Replay {
            return Ok(1);
        }
    }
    if let Some(b) = baseline {
        return metrics_cmd(&out_dir.join("results.jsonl"), Some(&b));
    }
    Ok(0)
}

fn live_gateway(
    p: &EvalPaths,
    cfg: &RunConfig,
) -> anyhow::Result<Arc<dyn model_gateway::ModelGateway>> {
    use model_gateway::adapters::anthropic::AnthropicAdapter;
    use model_gateway::adapters::openai::OpenAiAdapter;
    use model_gateway::ratelimit::local::LocalBuckets;
    use model_gateway::{
        DefaultRedactor, GatewayBuilder, PriceTable, RoutingFile, RoutingTable, TableRouter,
        TokenBucketLimiter,
    };
    let lookup = |k: &str| std::env::var(k).ok();
    let mut table = RoutingTable::default_table(&lookup)?;
    if let Some(o) = &cfg.routing_override {
        let text = std::fs::read_to_string(p.root.join(o))
            .with_context(|| format!("reading routing override {o}"))?;
        let file: RoutingFile = serde_yaml::from_str(&text)?;
        table = table.with_override(&file, &lookup)?;
    }
    let mut b = GatewayBuilder::new()
        .redactor(Arc::new(DefaultRedactor::new()))
        .router(Arc::new(TableRouter::new(table)))
        .prices(Arc::new(PriceTable::default_table()?))
        .rate_limiter(Arc::new(TokenBucketLimiter::new(Arc::new(
            LocalBuckets::new(),
        ))));
    let mut any = false;
    if let Some(a) = AnthropicAdapter::from_env()? {
        b = b.adapter(Arc::new(a));
        any = true;
    }
    if let Some(a) = OpenAiAdapter::from_env()? {
        b = b.adapter(Arc::new(a));
        any = true;
    }
    if !any {
        bail!("live mode needs ANTHROPIC_API_KEY or OPENAI_API_KEY");
    }
    Ok(Arc::new(b.build()?))
}

fn read_rows(results: &Path) -> anyhow::Result<Vec<result::CaseResult>> {
    let text = std::fs::read_to_string(results)
        .with_context(|| format!("reading {}", results.display()))?;
    metrics::parse_jsonl(&text).map_err(|e| anyhow::anyhow!("{}: {e}", results.display()))
}

fn read_baseline(path: &Path) -> anyhow::Result<metrics::Metrics> {
    let text = std::fs::read_to_string(path)?;
    let report: metrics::MetricsReport = serde_json::from_str(&text)
        .with_context(|| format!("parsing baseline {}", path.display()))?;
    Ok(report.overall)
}

fn metrics_cmd(results: &Path, baseline: Option<&Path>) -> anyhow::Result<i32> {
    let rows = read_rows(results)?;
    let m = metrics::compute(&rows);
    println!("{}", serde_json::to_string_pretty(&m)?);
    if let Some(b) = baseline {
        let regs = metrics::regressions(&m.overall, &read_baseline(b)?);
        for r in &regs {
            eprintln!(
                "regression: {} {:.4} vs baseline {:.4} ({})",
                r.metric, r.current, r.baseline, r.rule
            );
        }
        if !regs.is_empty() {
            return Ok(2);
        }
    }
    Ok(0)
}

async fn report_cmd(
    results: &Path,
    baseline: Option<&Path>,
    persist: bool,
    update_baseline: bool,
) -> anyhow::Result<i32> {
    let p = paths()?;
    let rows = read_rows(results)?;
    let manifest_path = results
        .parent()
        .map(|d| d.join("manifest.json"))
        .unwrap_or_else(|| PathBuf::from("manifest.json"));
    let manifest: RunManifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )?;
    let base = match baseline {
        Some(b) => Some(read_baseline(b)?),
        None => None,
    };
    let reviewer_versions = BTreeMap::from([(
        "correctness".to_owned(),
        reviewers::correctness::CORRECTNESS_REVIEWER_VERSION.to_owned(),
    )]);
    let r = report::build_report(&manifest, &rows, base.as_ref(), reviewer_versions);
    let (md, js) = report::write_report(&p.quality().join("reports"), &r)?;
    println!("{}\n{}", md.display(), js.display());
    if update_baseline {
        let path = report::update_baseline(&p.quality().join("baselines"), &rows, &r)?;
        println!("{}", path.display());
    }
    if persist {
        let url = std::env::var("DATABASE_URL").context("--persist needs DATABASE_URL")?;
        let pool = sqlx::PgPool::connect(&url).await?;
        let rel = md
            .strip_prefix(&p.root)
            .unwrap_or(&md)
            .to_string_lossy()
            .replace('\\', "/");
        match report::persist(&pool, &rows, &r, &rel).await {
            Ok(n) => println!("{n} model_eval_results rows inserted"),
            Err(e) => {
                eprintln!("persist failed (report files were written): {e:#}");
                return Ok(1);
            }
        }
    }
    Ok(if r.regressions.is_empty() { 0 } else { 2 })
}
