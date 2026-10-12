//! Evaluation runner (EVAL-003): every case × every repetition under one model configuration,
//! in replay or live mode, with bounded parallelism and one JSONL row per run.
//!
//! **Pipeline stand-in.** The review pipeline (PIPE-003), context engine (CTX-*) and verification
//! (VER-*) do not exist yet. Until they do, [`ReviewerExecutor`] runs the stage that exists: the
//! correctness reviewer on the case's hand-authored `context.json`, through the real gateway
//! (replay fixtures or live providers), followed by candidate normalisation. Every normalised
//! candidate is treated as published and every rejected one as suppressed; reports carry a banner
//! saying verification was not run. The [`ReviewExecutor`] seam is where `pipeline::local::
//! run_review` plugs in later.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use async_trait::async_trait;
use model_gateway::adapters::record::FIXTURE_ORG_ID;
use model_gateway::adapters::replay::{ReplayAdapter, ReplayConfig};
use model_gateway::fixture::FixtureStore;
use model_gateway::{
    CallBudget, DefaultRedactor, GatewayBuilder, GatewayError, ModelGateway, ModelTier,
    PermanentKind, PrivacyClass, ProviderId, RiskBand, RouteCandidate, StaticRouter, TenantScope,
    TraceContext,
};
use review_core::ids::RepositoryId;
use reviewers::focus::active_profiles;
use reviewers::{
    normalize_all, ContextSnapshot, CorrectnessReviewer, FocusProfile, ReviewContext,
    ReviewRequest, Reviewer, ReviewerError, ReviewerKind, RiskAssessment,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::case::LoadedCase;
use super::matcher::{match_case, Lineage, ProducedFinding, Stage};
use super::result::{CallStat, CaseResult, StructuredStats, UsageTotals};

/// Live runs above this estimate need `--yes`.
pub const LIVE_CONFIRM_THRESHOLD_USD: f64 = 5.0;
/// Output tokens of every eval reviewer call (part of the request hash).
pub const EVAL_MAX_OUTPUT_TOKENS: u32 = 4_000;
/// Placeholder model name the replay gateway routes to (fixtures use `any`).
pub const REPLAY_MODEL: &str = "replay";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalGatewayMode {
    Replay,
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub max_model_calls: u32,
    pub max_model_tokens: u64,
}

/// `benchmarks/quality/configs/*.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub name: String,
    pub gateway_mode: EvalGatewayMode,
    #[serde(default)]
    pub routing_override: Option<String>,
    pub reviewers: Vec<String>,
    pub verification_version: String,
    pub repetitions: u32,
    pub max_parallel_cases: usize,
    pub budgets: Budgets,
}

impl RunConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self =
            serde_yaml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if cfg.repetitions == 0 || cfg.max_parallel_cases == 0 {
            bail!("repetitions and max_parallel_cases must be at least 1");
        }
        Ok(cfg)
    }

    /// blake3 of the canonical config.
    pub fn hash(&self) -> String {
        let v = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        model_gateway::request_hash::hash_value(&v)
    }
}

/// `blake3(config_hash ‖ corpus_hash ‖ engine_git_sha ‖ started_at)`.
pub fn run_id(config_hash: &str, corpus_hash: &str, git_sha: &str, started_at: &str) -> String {
    let mut h = blake3::Hasher::new();
    for part in [config_hash, corpus_hash, git_sha, started_at] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// Live mode prints an up-front estimate and requires `--yes` above $5.
pub fn confirm_live(estimated_usd: f64, yes: bool) -> anyhow::Result<()> {
    if estimated_usd > LIVE_CONFIRM_THRESHOLD_USD && !yes {
        bail!(
            "estimated live cost ${estimated_usd:.2} exceeds ${LIVE_CONFIRM_THRESHOLD_USD:.2}; rerun with --yes"
        );
    }
    Ok(())
}

/// `cases × repetitions × avg_tokens × price per token`.
pub fn estimate_live_usd(
    cases: usize,
    repetitions: u32,
    avg_tokens: u64,
    usd_per_mtok: f64,
) -> f64 {
    cases as f64 * f64::from(repetitions) * avg_tokens as f64 * usd_per_mtok / 1_000_000.0
}

/// What one case run produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaseRun {
    pub findings: Vec<ProducedFinding>,
    pub usage: UsageTotals,
    pub structured: StructuredStats,
    pub context_symbols: Vec<String>,
    pub calls: Vec<CallStat>,
    pub degraded: bool,
}

/// Why a case produced no usable run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseFailure {
    ReplayMiss(Vec<String>),
    NoContext,
    Error(String),
}

impl CaseFailure {
    pub fn class(&self) -> &'static str {
        match self {
            Self::ReplayMiss(_) => "replay_miss",
            Self::NoContext => "no_context",
            Self::Error(_) => "error",
        }
    }
}

/// Runs the review of one case. Implemented by the reviewer stand-in below and, later, by the
/// in-process pipeline.
#[async_trait]
pub trait ReviewExecutor: Send + Sync {
    async fn execute(&self, case: &LoadedCase, repetition: u32) -> Result<CaseRun, CaseFailure>;
}

/// The reviewer inputs of a case, exactly as the executor and the fixture helper build them.
#[derive(Debug, Clone)]
pub struct CaseInputs {
    pub context: ContextSnapshot,
    pub risk: RiskAssessment,
    pub focus: Vec<FocusProfile>,
    pub trace: TraceContext,
    pub tenant: TenantScope,
}

impl CaseInputs {
    pub fn of(case: &LoadedCase) -> Option<Self> {
        let context = case.context.clone()?;
        let signals: Vec<String> = context
            .change_summary
            .change_classes
            .iter()
            .chain(context.risk_signals.iter().map(|r| &r.signal))
            .cloned()
            .collect();
        let focus = active_profiles(signals.iter().map(String::as_str));
        let repo = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, case.case.id.as_bytes());
        Some(Self {
            context,
            risk: RiskAssessment {
                level: RiskBand::Medium,
                modules_touched: 1,
                signals,
            },
            focus,
            trace: TraceContext::default(),
            tenant: TenantScope {
                organization_id: FIXTURE_ORG_ID,
                repository_id: RepositoryId::from_uuid(repo),
            },
        })
    }

    /// A fixed budget: `max_output_tokens` is part of the request hash.
    pub fn budget() -> CallBudget {
        CallBudget {
            max_input_tokens: 200_000,
            max_output_tokens: EVAL_MAX_OUTPUT_TOKENS,
            max_cost_usd_micros: None,
            deadline: tokio::time::Instant::now() + Duration::from_secs(300),
            remaining_fraction: 1.0,
            max_attempts: 3,
        }
    }

    pub fn request(&self) -> ReviewRequest<'_> {
        ReviewRequest {
            context: &self.context,
            risk: &self.risk,
            cluster_id: "cluster-1",
            focus: &self.focus,
            tenant: self.tenant,
            budget: Self::budget(),
            trace: &self.trace,
            privacy: PrivacyClass::Standard,
        }
    }
}

/// A gateway that serves only replay fixtures from `fixtures_dir`.
pub fn replay_gateway(fixtures_dir: &Path) -> anyhow::Result<Arc<dyn ModelGateway>> {
    let store = Arc::new(FixtureStore::new(fixtures_dir));
    let mut cfg = ReplayConfig::new(fixtures_dir);
    cfg.miss_log = None;
    let candidate = RouteCandidate {
        provider: ProviderId::new(ProviderId::REPLAY),
        model: REPLAY_MODEL.into(),
        max_context: 1_000_000,
        supports_reasoning: true,
    };
    let mut router = StaticRouter::new();
    for tier in [
        ModelTier::Classifier,
        ModelTier::FastReasoner,
        ModelTier::ReviewReasoner,
        ModelTier::Verifier,
        ModelTier::DeepReasoner,
    ] {
        router = router.with_route(tier, vec![candidate.clone()]);
    }
    let gw = GatewayBuilder::new()
        .redactor(Arc::new(DefaultRedactor::new()))
        .adapter(Arc::new(ReplayAdapter::new(
            ProviderId::new(ProviderId::REPLAY),
            store,
            cfg,
        )))
        .router(Arc::new(router))
        .build()?;
    Ok(Arc::new(gw))
}

fn produced(c: &reviewers::NormalizedCandidate) -> ProducedFinding {
    ProducedFinding {
        id: format!("c{:02}", c.ordinal),
        stage: Stage::Published,
        category: c.category.as_str().to_owned(),
        severity: c.severity,
        primary_symbol: Some(c.anchor.symbol_id.as_str().to_owned()),
        affected_symbols: c
            .affected_symbols
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect(),
        path: c.anchor.path.clone(),
        start_line: c.anchor.start_line,
        end_line: c.anchor.end_line,
        title: c.title.clone(),
        claim: c.claim.clone(),
        description: c.description.clone(),
    }
}

fn rejected(r: &reviewers::Rejection) -> ProducedFinding {
    let text = |k: &str| {
        r.raw_json
            .get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    ProducedFinding {
        id: format!("c{:02}", r.ordinal),
        stage: Stage::Suppressed("rejected_invalid".into()),
        category: text("category"),
        severity: review_core::finding::Severity::parse(&text("severity"))
            .unwrap_or(review_core::finding::Severity::Info),
        primary_symbol: None,
        affected_symbols: Vec::new(),
        path: String::new(),
        start_line: 0,
        end_line: 0,
        title: text("title"),
        claim: text("claim"),
        description: text("description"),
    }
}

/// The reviewer-stage stand-in for the pipeline (see the module docs).
pub struct ReviewerExecutor {
    gateway: Arc<dyn ModelGateway>,
    reviewer: CorrectnessReviewer,
    reviewers: BTreeSet<String>,
}

impl std::fmt::Debug for ReviewerExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewerExecutor")
            .field("reviewers", &self.reviewers)
            .finish_non_exhaustive()
    }
}

impl ReviewerExecutor {
    pub fn new(gateway: Arc<dyn ModelGateway>, reviewers: &[String]) -> anyhow::Result<Self> {
        Ok(Self {
            gateway,
            reviewer: CorrectnessReviewer::new()?,
            reviewers: reviewers.iter().cloned().collect(),
        })
    }
}

#[async_trait]
impl ReviewExecutor for ReviewerExecutor {
    async fn execute(&self, case: &LoadedCase, _repetition: u32) -> Result<CaseRun, CaseFailure> {
        let inputs = CaseInputs::of(case).ok_or(CaseFailure::NoContext)?;
        let mut run = CaseRun {
            context_symbols: inputs
                .context
                .changed_symbols()
                .iter()
                .map(|s| s.symbol_id.clone())
                .chain(inputs.context.nodes().iter().map(|n| n.symbol_id.clone()))
                .collect(),
            ..CaseRun::default()
        };
        if !self.reviewers.contains("correctness") {
            return Ok(run);
        }
        // Generated code is never sent to a reviewer (REV-002): a cluster of generated symbols
        // only makes no model call.
        let changed = inputs.context.changed_symbols();
        if !changed.is_empty() && changed.iter().all(|s| s.generated) {
            return Ok(run);
        }
        let req = inputs.request();
        match self
            .reviewer
            .review(req, self.gateway.as_ref(), CancellationToken::new())
            .await
        {
            Ok(out) => {
                let meta = &out.model_response_meta;
                run.structured.success += 1;
                run.usage = UsageTotals {
                    model_calls: u64::from(meta.attempts.max(1)),
                    input_tokens: u64::from(meta.usage.input_uncached),
                    output_tokens: u64::from(meta.usage.output),
                    cached_tokens: u64::from(meta.usage.cache_read + meta.usage.cache_write),
                    cost_usd_micros: meta.cost_usd_micros.unwrap_or(0),
                };
                run.calls.push(CallStat {
                    task: "correctness_review".into(),
                    tier: self.reviewer.tier(&inputs.risk).as_str().to_owned(),
                    provider: meta.provider.clone(),
                    model: meta.model.clone(),
                    routing_table_hash: meta.route_table_hash.clone(),
                    prompt_version: self.reviewer.prompt().prompt_version(),
                    calls: u64::from(meta.attempts.max(1)),
                    input_tokens: run.usage.input_tokens,
                    output_tokens: run.usage.output_tokens,
                    cost_usd_micros: run.usage.cost_usd_micros,
                });
                let (ok, rejected_items) =
                    normalize_all(&out.raw, &out.ref_table, ReviewerKind::Correctness);
                run.findings.extend(ok.iter().map(produced));
                run.findings.extend(rejected_items.iter().map(rejected));
                Ok(run)
            }
            Err(ReviewerError::Gateway(GatewayError::Permanent {
                kind: PermanentKind::ReplayMiss,
                detail,
                ..
            })) => Err(CaseFailure::ReplayMiss(vec![detail])),
            Err(ReviewerError::Gateway(GatewayError::SchemaViolation { .. })) => {
                run.structured.failures_after_repair += 1;
                *run.structured
                    .failures_by_task
                    .entry("correctness_review".into())
                    .or_default() += 1;
                run.degraded = true;
                Ok(run)
            }
            Err(ReviewerError::Gateway(e)) => {
                run.degraded = true;
                tracing::warn!(case = %case.case.id, error = %e, "reviewer call failed");
                Ok(run)
            }
            Err(e) => Err(CaseFailure::Error(e.to_string())),
        }
    }
}

/// Runs every case × repetition through `executor`. Results are sorted by (case, repetition).
pub async fn run_cases(
    cases: &[LoadedCase],
    executor: Arc<dyn ReviewExecutor>,
    cfg: &RunConfig,
    run_id: &str,
) -> Vec<CaseResult> {
    let semaphore = Arc::new(Semaphore::new(cfg.max_parallel_cases.max(1)));
    let enabled: BTreeSet<&str> = cfg.reviewers.iter().map(String::as_str).collect();
    let mut set: JoinSet<CaseResult> = JoinSet::new();
    let mut ids = std::collections::HashMap::new();
    for case in cases {
        for repetition in 0..cfg.repetitions {
            let case = case.clone();
            let executor = Arc::clone(&executor);
            let semaphore = Arc::clone(&semaphore);
            let required_ok = case
                .case
                .required_reviewers
                .iter()
                .all(|r| enabled.contains(r.as_str()));
            let base = blank_result(&case, cfg, run_id, repetition, required_ok);
            let fallback = base.clone();
            let handle = set.spawn(async move {
                let Ok(_permit) = semaphore.acquire_owned().await else {
                    return CaseResult {
                        case_error: Some("cancelled".into()),
                        ..base
                    };
                };
                let started = tokio::time::Instant::now();
                let outcome = executor.execute(&case, repetition).await;
                let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                finish(case, base, outcome, latency_ms)
            });
            ids.insert(handle.id(), fallback);
        }
    }
    let mut results = Vec::new();
    while let Some(joined) = set.join_next_with_id().await {
        match joined {
            Ok((_, r)) => results.push(r),
            Err(e) => {
                if let Some(base) = ids.remove(&e.id()) {
                    let class = if e.is_panic() { "panic" } else { "cancelled" };
                    results.push(CaseResult {
                        case_error: Some(class.into()),
                        ..base
                    });
                }
            }
        }
    }
    results.sort_by(|a, b| {
        a.case_id
            .cmp(&b.case_id)
            .then(a.repetition.cmp(&b.repetition))
    });
    results
}

fn blank_result(
    case: &LoadedCase,
    cfg: &RunConfig,
    run_id: &str,
    repetition: u32,
    required_ok: bool,
) -> CaseResult {
    CaseResult {
        run_id: run_id.to_owned(),
        config_name: cfg.name.clone(),
        case_id: case.case.id.clone(),
        repetition,
        classes: case.case.classes.clone(),
        required_reviewers_enabled: required_ok,
        synthetic_fixtures: case.case.synthetic_fixtures,
        case_error: None,
        replay_misses: Vec::new(),
        degraded: false,
        findings: Vec::new(),
        matches: None,
        usage: UsageTotals::default(),
        structured: StructuredStats::default(),
        latency_ms: 0,
        context_symbols: Vec::new(),
        calls: Vec::new(),
    }
}

fn finish(
    case: LoadedCase,
    base: CaseResult,
    outcome: Result<CaseRun, CaseFailure>,
    latency_ms: u64,
) -> CaseResult {
    match outcome {
        Ok(run) => {
            let matches = match_case(&case.case, &run.findings, &Lineage::new());
            CaseResult {
                degraded: run.degraded,
                findings: run.findings,
                matches: Some(matches),
                usage: run.usage,
                structured: run.structured,
                latency_ms,
                context_symbols: run.context_symbols,
                calls: run.calls,
                ..base
            }
        }
        Err(f) => CaseResult {
            case_error: Some(f.class().to_owned()),
            replay_misses: match &f {
                CaseFailure::ReplayMiss(h) => h.clone(),
                _ => Vec::new(),
            },
            latency_ms,
            ..base
        },
    }
}

/// Writes results as JSONL (one row per line, deterministic order).
pub fn write_jsonl(path: &Path, rows: &[CaseResult]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = String::new();
    for r in rows {
        text.push_str(&serde_json::to_string(r)?);
        text.push('\n');
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}
