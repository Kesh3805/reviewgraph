//! Gateway core and its builder (GW-001).
//!
//! The core pipeline order is fixed (GW-010): **redact → hash → cache → route → limit → send →
//! validate → account → emit**. Redaction runs on the caller's task before any I/O and cannot be
//! bypassed: a gateway cannot be built without a [`PreSendRedactor`].

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::field::Empty;
use tracing::Instrument;
use uuid::Uuid;

use crate::accounting::{LedgerRecord, LedgerSink, NoLedger, PriceTable};
use crate::adapter::{ProviderAdapter, ProviderRequest, ProviderResponse};
use crate::budget::{precheck_cost, precheck_tokens, WorstCasePricer};
use crate::cache::{cache_key, CacheEntry, ResponseCache};
use crate::error::{BudgetKind, Error, GatewayError, PermanentKind};
use crate::ratelimit::{LimitRequest, ModelLimits, NoLimit, RateLimiter};
use crate::redact::{PreSendRedactor, RedactionReport};
use crate::request_hash::request_hash;
use crate::retry::{retry, RetryOutcome, RetryPolicy, MAX_ATTEMPTS};
use crate::telemetry::{CallLabels, GatewayMetrics};
use crate::types::{
    AttemptedCandidate, CachePolicy, FinishReason, ModelOutput, ModelRequest, ModelResponse,
    ModelTier, OutputSchema, OutputValidator, PrivacyClass, ProviderId, RepairTurn, RequestHash,
    RiskBand, RouteCandidate, RouteDecision, SchemaErrorSummary, ServedFrom, StructuredInput,
    Usage,
};
use crate::validate::{cap_errors, SchemaValidators};

/// Upper bound for one provider HTTP attempt.
const MAX_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// The single entry point for model calls. Shared as `Arc<dyn ModelGateway>`.
#[async_trait]
pub trait ModelGateway: Send + Sync {
    async fn call(
        &self,
        req: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelResponse, GatewayError>;
}

/// Inputs of a routing decision.
#[derive(Debug, Clone)]
pub struct RouteQuery {
    pub tier: ModelTier,
    pub risk_band: RiskBand,
    pub privacy: PrivacyClass,
    pub est_input_tokens: u32,
    pub max_output_tokens: u32,
    pub remaining_budget_fraction: f32,
    /// Providers whose strict-schema check passes for this request's schema.
    pub schema_strict_ok: HashSet<ProviderId>,
}

/// Chooses the ordered candidates for a call.
pub trait RouteSource: Send + Sync {
    fn route(
        &self,
        registered: &HashSet<ProviderId>,
        query: &RouteQuery,
    ) -> Result<RouteDecision, GatewayError>;

    /// Whether `provider` may receive a request of this privacy class. The gateway asserts it
    /// again right before every send (defence in depth).
    fn permits(&self, _provider: &ProviderId, _privacy: PrivacyClass) -> bool {
        true
    }

    /// Configured account limits for a routed model, if any (GW-007).
    fn limits(&self, _provider: &ProviderId, _model: &str) -> Option<ModelLimits> {
        None
    }
}

/// A fixed tier-to-candidates map. Used by tests and by deployments without a routing table.
#[derive(Debug, Clone, Default)]
pub struct StaticRouter {
    routes: HashMap<ModelTier, Vec<RouteCandidate>>,
}

impl StaticRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_route(mut self, tier: ModelTier, candidates: Vec<RouteCandidate>) -> Self {
        self.routes.insert(tier, candidates);
        self
    }
}

impl RouteSource for StaticRouter {
    fn route(
        &self,
        registered: &HashSet<ProviderId>,
        q: &RouteQuery,
    ) -> Result<RouteDecision, GatewayError> {
        let candidates: Vec<RouteCandidate> = self
            .routes
            .get(&q.tier)
            .into_iter()
            .flatten()
            .filter(|c| registered.contains(&c.provider))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return Err(GatewayError::NoEligibleProvider {
                tier: q.tier,
                privacy: q.privacy,
                reason: "no registered provider for tier".into(),
            });
        }
        Ok(RouteDecision {
            requested_tier: q.tier,
            effective_tier: q.tier,
            candidates,
            downgraded: None,
            table_hash: "static".into(),
            attempted: Vec::new(),
        })
    }
}

/// Conservative token estimate used for budget and context-window checks.
pub fn estimate_input_tokens(req: &ModelRequest) -> u32 {
    let bytes = req.input.byte_len() as f64;
    (bytes / 3.5).ceil().min(f64::from(u32::MAX)) as u32
}

/// Builder state before a redactor is set: `build()` does not exist yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeedsRedactor;

/// Composes adapters, router, redactor and the other gateway parts.
///
/// A redactor is mandatory (GW-010 `builder_requires_redactor`): without one there is no
/// `build()`, so this does not compile:
///
/// ```compile_fail
/// let _ = model_gateway::GatewayBuilder::new().build();
/// ```
pub struct GatewayBuilder<R = NeedsRedactor> {
    adapters: Vec<Arc<dyn ProviderAdapter>>,
    router: Option<Arc<dyn RouteSource>>,
    retry: Option<RetryPolicy>,
    limiter: Option<Arc<dyn RateLimiter>>,
    pricer: Option<Arc<dyn WorstCasePricer>>,
    prices: Option<Arc<PriceTable>>,
    ledger: Option<Arc<dyn LedgerSink>>,
    cache: Option<Arc<dyn ResponseCache>>,
    schemas: Vec<OutputSchema>,
    metrics: Option<GatewayMetrics>,
    redactor: R,
}

impl Default for GatewayBuilder<NeedsRedactor> {
    fn default() -> Self {
        Self {
            adapters: Vec::new(),
            router: None,
            retry: None,
            limiter: None,
            pricer: None,
            prices: None,
            ledger: None,
            cache: None,
            schemas: Vec::new(),
            metrics: None,
            redactor: NeedsRedactor,
        }
    }
}

impl<R> std::fmt::Debug for GatewayBuilder<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayBuilder")
            .field("adapters", &self.adapters.len())
            .field("schemas", &self.schemas.len())
            .finish_non_exhaustive()
    }
}

impl GatewayBuilder<NeedsRedactor> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<R> GatewayBuilder<R> {
    /// The mandatory pre-send redactor.
    pub fn redactor(
        self,
        redactor: Arc<dyn PreSendRedactor>,
    ) -> GatewayBuilder<Arc<dyn PreSendRedactor>> {
        GatewayBuilder {
            adapters: self.adapters,
            router: self.router,
            retry: self.retry,
            limiter: self.limiter,
            pricer: self.pricer,
            prices: self.prices,
            ledger: self.ledger,
            cache: self.cache,
            schemas: self.schemas,
            metrics: self.metrics,
            redactor,
        }
    }

    pub fn adapter(mut self, adapter: Arc<dyn ProviderAdapter>) -> Self {
        self.adapters.push(adapter);
        self
    }

    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = Some(policy);
        self
    }

    pub fn rate_limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// Worst-case pricing for the cost pre-check. Defaults to the price table when one is set.
    pub fn pricer(mut self, pricer: Arc<dyn WorstCasePricer>) -> Self {
        self.pricer = Some(pricer);
        self
    }

    /// Price table used for `cost_usd_micros` and the ledger.
    pub fn prices(mut self, prices: Arc<PriceTable>) -> Self {
        self.prices = Some(prices);
        self
    }

    /// Per-attempt ledger sink (`model_calls`).
    pub fn ledger(mut self, ledger: Arc<dyn LedgerSink>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Tenant-scoped response cache for cache-allowed tasks.
    pub fn response_cache(mut self, cache: Arc<dyn ResponseCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn router(mut self, router: Arc<dyn RouteSource>) -> Self {
        self.router = Some(router);
        self
    }

    /// Registers an output schema for the startup self-test: `build()` compiles every registered
    /// schema, so a broken schema surfaces at boot instead of at the first call (GW-009).
    pub fn register_schema(mut self, schema: OutputSchema) -> Self {
        self.schemas.push(schema);
        self
    }

    /// Metric instruments; defaults to instruments on the global meter provider.
    pub fn metrics(mut self, metrics: GatewayMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }
}

impl GatewayBuilder<Arc<dyn PreSendRedactor>> {
    pub fn build(self) -> Result<Gateway, Error> {
        let router = self
            .router
            .ok_or_else(|| Error::Config("a router is required".into()))?;
        let adapters: HashMap<ProviderId, Arc<dyn ProviderAdapter>> = self
            .adapters
            .into_iter()
            .map(|a| (a.provider(), a))
            .collect();
        if adapters.is_empty() {
            return Err(Error::Config("at least one adapter is required".into()));
        }
        let validators = SchemaValidators::new();
        for schema in &self.schemas {
            validators.compile(schema).map_err(|e| {
                Error::Config(format!(
                    "schema self-test failed for `{}` v{}: {e}",
                    schema.name, schema.version
                ))
            })?;
        }
        let pricer = self
            .pricer
            .or_else(|| self.prices.clone().map(|p| p as Arc<dyn WorstCasePricer>));
        Ok(Gateway {
            adapters,
            router,
            retry: self.retry.unwrap_or_default(),
            limiter: self.limiter.unwrap_or_else(|| Arc::new(NoLimit)),
            pricer,
            prices: self.prices,
            ledger: self.ledger.unwrap_or_else(|| Arc::new(NoLedger)),
            cache: self.cache,
            redactor: self.redactor,
            validators,
            metrics: self.metrics.unwrap_or_else(GatewayMetrics::global),
        })
    }
}

/// The gateway core. Holds no locks across `.await`.
pub struct Gateway {
    adapters: HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
    router: Arc<dyn RouteSource>,
    retry: RetryPolicy,
    limiter: Arc<dyn RateLimiter>,
    pricer: Option<Arc<dyn WorstCasePricer>>,
    prices: Option<Arc<PriceTable>>,
    ledger: Arc<dyn LedgerSink>,
    cache: Option<Arc<dyn ResponseCache>>,
    redactor: Arc<dyn PreSendRedactor>,
    validators: SchemaValidators,
    metrics: GatewayMetrics,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway")
            .field("providers", &self.adapters.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Facts about one attempt, turned into a ledger row.
struct AttemptFacts<'a> {
    req: &'a ModelRequest,
    hash: &'a RequestHash,
    candidate: &'a RouteCandidate,
    attempt: u8,
    served_from: ServedFrom,
    outcome: String,
    usage: Usage,
    cost: Option<u64>,
    latency_ms: u32,
}

/// One request sent to one route candidate (the original request or its repair).
#[derive(Clone, Copy)]
struct Leg<'a> {
    req: &'a ModelRequest,
    hash: &'a RequestHash,
    candidate: &'a RouteCandidate,
    adapter: &'a dyn ProviderAdapter,
    limit: Option<&'a LimitRequest>,
}

fn finish_str(f: &FinishReason) -> &'static str {
    match f {
        FinishReason::Complete => "complete",
        FinishReason::MaxTokens => "max_tokens",
        FinishReason::Refusal => "refusal",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Other(_) => "other",
    }
}

fn add_usage(a: Usage, b: Usage) -> Usage {
    Usage {
        input_uncached: a.input_uncached.saturating_add(b.input_uncached),
        cache_write: a.cache_write.saturating_add(b.cache_write),
        cache_read: a.cache_read.saturating_add(b.cache_read),
        output: a.output.saturating_add(b.output),
        reasoning: a.reasoning.saturating_add(b.reasoning),
    }
}

fn truncated_error() -> SchemaErrorSummary {
    SchemaErrorSummary {
        instance_path: String::new(),
        keyword: "max_tokens".into(),
        message: "output truncated at the token limit; not repaired".into(),
    }
}

fn not_json_error() -> SchemaErrorSummary {
    SchemaErrorSummary {
        instance_path: String::new(),
        keyword: "type".into(),
        message: "structured JSON output expected".into(),
    }
}

impl Gateway {
    fn cost_of(
        &self,
        candidate: &RouteCandidate,
        reported_model: &str,
        usage: &Usage,
    ) -> Option<u64> {
        let prices = self.prices.as_ref()?;
        prices
            .cost_micros(candidate.provider.as_str(), &candidate.model, usage)
            .or_else(|| prices.cost_micros(candidate.provider.as_str(), reported_model, usage))
    }

    fn record(&self, f: AttemptFacts<'_>) {
        let prices_as_of = self
            .prices
            .as_ref()
            .and_then(|p| p.as_of(f.candidate.provider.as_str(), &f.candidate.model));
        self.ledger.record(LedgerRecord {
            id: Uuid::now_v7(),
            organization_id: f.req.tenant.organization_id,
            repository_id: f.req.tenant.repository_id,
            review_run_id: f.req.trace.review_run_id,
            reviewer_run_id: f.req.trace.reviewer_run_id,
            task: f.req.task.as_str().to_owned(),
            tier: f.req.tier.as_str().to_owned(),
            provider: f.candidate.provider.0.clone(),
            model: f.candidate.model.clone(),
            attempt: i16::from(f.attempt),
            request_hash: f.hash.0.clone(),
            served_from: f.served_from,
            outcome: f.outcome,
            usage: f.usage,
            cost_usd_micros: f.cost,
            latency_ms: f.latency_ms,
            prices_as_of,
        });
    }

    /// Cache TTL when this request may use the response cache.
    fn cache_ttl(&self, req: &ModelRequest) -> Option<Duration> {
        match (&self.cache, req.cache) {
            (Some(_), CachePolicy::PromptAndResponse { ttl }) if req.task.response_cacheable() => {
                Some(ttl)
            }
            _ => None,
        }
    }

    fn labels(req: &ModelRequest, candidate: &RouteCandidate) -> CallLabels {
        CallLabels {
            provider: candidate.provider.0.clone(),
            model: candidate.model.clone(),
            task: req.task.as_str(),
            tier: req.tier.as_str(),
        }
    }

    /// Runs the redactor; a panic or a blocked report fails the call before any I/O.
    fn redact(&self, input: &mut StructuredInput) -> Result<RedactionReport, GatewayError> {
        let redactor = &self.redactor;
        let report = catch_unwind(AssertUnwindSafe(|| redactor.redact(input))).map_err(|_| {
            self.metrics.blocked("redactor_failure");
            GatewayError::Permanent {
                kind: PermanentKind::InvalidRequest,
                provider: None,
                detail: "pre-send redaction failed; the request was not sent".into(),
            }
        })?;
        self.metrics.redactions(&report.by_pattern);
        if report.blocked {
            self.metrics.blocked("secret_in_system_context");
            return Err(GatewayError::Permanent {
                kind: PermanentKind::InvalidRequest,
                provider: None,
                detail: "secret material found in system context; the request was not sent".into(),
            });
        }
        Ok(report)
    }

    /// Schema errors plus the caller's semantic errors, capped (GW-009).
    fn check(
        &self,
        schema: &OutputSchema,
        semantic: Option<&dyn OutputValidator>,
        output: &Value,
    ) -> Result<Vec<SchemaErrorSummary>, GatewayError> {
        let mut errors = self.validators.validate(schema, output)?;
        if let Some(v) = semantic {
            errors.extend(v.validate(output));
        }
        Ok(cap_errors(errors))
    }

    /// One provider attempt: limit, send, reconcile and account.
    async fn send_once(
        &self,
        leg: Leg<'_>,
        attempt: u8,
        cancel: &CancellationToken,
    ) -> Result<ProviderResponse, GatewayError> {
        let candidate = leg.candidate;
        let span = tracing::info_span!(
            "model_request.attempt",
            "rg.attempt" = attempt,
            "gen_ai.system" = candidate.provider.as_str(),
            "gen_ai.request.model" = candidate.model.as_str(),
            "rg.outcome" = Empty,
        );
        let fut = async {
            let reservation = match leg.limit {
                Some(lr) => match self.limiter.acquire(lr, cancel).await {
                    Ok(r) => Some(r),
                    Err(e) => {
                        if matches!(e, GatewayError::RateLimited { .. }) {
                            self.metrics.rate_limited(candidate.provider.as_str());
                        }
                        return Err(e);
                    }
                },
                None => None,
            };
            let remaining = leg
                .req
                .budget
                .deadline
                .saturating_duration_since(Instant::now());
            let provider_req = ProviderRequest {
                request: leg.req,
                candidate,
                request_hash: leg.hash,
                timeout: remaining.min(MAX_ATTEMPT_TIMEOUT),
            };
            let attempt_started = Instant::now();
            let result = leg.adapter.send(&provider_req).await;
            let latency_ms =
                u32::try_from(attempt_started.elapsed().as_millis()).unwrap_or(u32::MAX);
            match result {
                Ok(resp) => {
                    let u = resp.usage;
                    if let Some(r) = &reservation {
                        let input = u.input_uncached + u.cache_write + u.cache_read;
                        self.limiter.reconcile(r, input, u.output).await;
                    }
                    self.record(AttemptFacts {
                        req: leg.req,
                        hash: leg.hash,
                        candidate,
                        attempt,
                        served_from: resp.served_from,
                        outcome: "ok".into(),
                        usage: u,
                        cost: self.cost_of(candidate, &resp.model, &u),
                        latency_ms,
                    });
                    Ok(resp)
                }
                Err(e) => {
                    if matches!(e, GatewayError::RateLimited { .. }) {
                        self.metrics.rate_limited(candidate.provider.as_str());
                    }
                    self.record(AttemptFacts {
                        req: leg.req,
                        hash: leg.hash,
                        candidate,
                        attempt,
                        served_from: ServedFrom::Live,
                        outcome: e.class().to_owned(),
                        usage: Usage::default(),
                        cost: None,
                        latency_ms,
                    });
                    Err(e)
                }
            }
        };
        let result = fut.instrument(span.clone()).await;
        span.record(
            "rg.outcome",
            match &result {
                Ok(_) => "ok",
                Err(e) => e.class(),
            },
        );
        result
    }

    /// Retries one leg on its candidate. Ledger attempt numbers start after `offset`.
    async fn run_leg(
        &self,
        leg: Leg<'_>,
        offset: u8,
        max_attempts: u8,
        cancel: &CancellationToken,
    ) -> RetryOutcome<ProviderResponse> {
        let mut budget = leg.req.budget;
        budget.max_attempts = max_attempts;
        let outcome = retry(&self.retry, &budget, cancel, move |attempt| {
            self.send_once(leg, offset.saturating_add(attempt), cancel)
        })
        .await;
        self.metrics.retries(
            &Self::labels(leg.req, leg.candidate),
            u64::from(outcome.attempts.saturating_sub(1)),
        );
        outcome
    }

    /// Validates a structured output and sends at most one repair turn on the same candidate
    /// (GW-009). `used` is the number of attempts this candidate already consumed; the repair
    /// counts against the same per-call attempt budget.
    async fn validate_and_repair(
        &self,
        leg: Leg<'_>,
        resp: ProviderResponse,
        used: u8,
        attempts_total: &mut u8,
        cancel: &CancellationToken,
        span: &tracing::Span,
    ) -> Result<ProviderResponse, GatewayError> {
        let req = leg.req;
        let Some(schema) = &req.output_schema else {
            return Ok(resp);
        };
        let task = req.task.as_str();
        let provider = leg.candidate.provider.as_str();
        if resp.finish_reason == FinishReason::MaxTokens {
            // Truncated output: a repair would truncate too. The caller may re-plan.
            self.metrics.structured_failure(task, provider, "first");
            return Err(GatewayError::SchemaViolation {
                errors: vec![truncated_error()],
                repaired: false,
            });
        }
        let previous = match &resp.output {
            ModelOutput::Json(v) => v.clone(),
            // Refusals and filtered responses carry text; the caller handles them.
            ModelOutput::Text(_) => return Ok(resp),
        };
        let errors = self.check(schema, req.validator.as_deref(), &previous)?;
        if errors.is_empty() {
            self.metrics.structured_success(task, provider);
            return Ok(resp);
        }
        self.metrics.structured_failure(task, provider, "first");
        if req.input.repair.is_some() {
            return Err(GatewayError::SchemaViolation {
                errors,
                repaired: true,
            });
        }
        let remaining = req
            .budget
            .max_attempts
            .clamp(1, MAX_ATTEMPTS)
            .saturating_sub(used);
        if remaining == 0 {
            return Err(GatewayError::SchemaViolation {
                errors,
                repaired: false,
            });
        }

        let mut repaired = req.clone();
        repaired.input.repair = Some(RepairTurn {
            previous_output: previous,
            errors,
            tool_use_id: resp.tool_use_id.clone(),
        });
        // The previous output can echo secrets from the input; it is redacted like the rest.
        self.redact(&mut repaired.input)?;
        let repair_hash = request_hash(&repaired);
        self.metrics.schema_repair(task, provider);
        span.record("rg.repair", true);
        let repair_leg = Leg {
            req: &repaired,
            hash: &repair_hash,
            ..leg
        };
        let second = self.run_leg(repair_leg, used, remaining, cancel).await;
        *attempts_total = attempts_total.saturating_add(second.attempts);
        let resp2 = second.result?;
        let errors2 = if resp2.finish_reason == FinishReason::MaxTokens {
            vec![truncated_error()]
        } else {
            match &resp2.output {
                ModelOutput::Json(v) => self.check(schema, req.validator.as_deref(), v)?,
                ModelOutput::Text(_) => vec![not_json_error()],
            }
        };
        if !errors2.is_empty() {
            self.metrics
                .structured_failure(task, provider, "after_repair");
            return Err(GatewayError::SchemaViolation {
                errors: errors2,
                repaired: true,
            });
        }
        self.metrics.structured_success(task, provider);
        Ok(ProviderResponse {
            usage: add_usage(resp.usage, resp2.usage),
            ..resp2
        })
    }

    async fn call_inner(
        &self,
        mut req: ModelRequest,
        cancel: CancellationToken,
        span: &tracing::Span,
    ) -> Result<ModelResponse, GatewayError> {
        let now = Instant::now();
        if req.budget.deadline <= now {
            return Err(GatewayError::BudgetExceeded {
                kind: BudgetKind::Deadline,
            });
        }
        if cancel.is_cancelled() {
            return Err(GatewayError::Cancelled);
        }
        // redact → hash: the hash describes exactly what leaves the process.
        let report = self.redact(&mut req.input)?;
        span.record("rg.redactions", report.replacements);
        let hash = request_hash(&req);
        span.record("rg.request_hash", hash.as_str());

        let est_input = estimate_input_tokens(&req);
        precheck_tokens(&req, est_input)?;
        let registered: HashSet<ProviderId> = self.adapters.keys().cloned().collect();
        let mut strict_ok = registered.clone();
        if let Some(schema) = &req.output_schema {
            if crate::schema_strict::check_strict_compatible(&schema.schema).is_err() {
                strict_ok.remove(&ProviderId::new(ProviderId::OPENAI));
            }
        }
        let query = RouteQuery {
            tier: req.tier,
            risk_band: req.risk_band,
            privacy: req.privacy,
            est_input_tokens: est_input,
            max_output_tokens: req.max_output_tokens,
            remaining_budget_fraction: req.budget.remaining_fraction,
            schema_strict_ok: strict_ok,
        };
        let mut route = self.router.route(&registered, &query)?;
        span.record("rg.effective_tier", route.effective_tier.as_str());

        // Response cache: after routing (provider and model are known), before rate limiting.
        let cache_ttl = self.cache_ttl(&req);
        if let (Some(cache), Some(first), Some(_)) =
            (&self.cache, route.candidates.first(), cache_ttl)
        {
            let key = cache_key_for(&req, &hash, first);
            match cache.get(req.tenant.organization_id, &key).await {
                Ok(Some(entry)) => {
                    self.metrics.cache_hit(req.task.as_str());
                    self.record(AttemptFacts {
                        req: &req,
                        hash: &hash,
                        candidate: first,
                        attempt: 1,
                        served_from: ServedFrom::ResponseCache,
                        outcome: "ok".into(),
                        usage: Usage::default(),
                        cost: Some(0),
                        latency_ms: 0,
                    });
                    span.record("gen_ai.system", first.provider.as_str());
                    span.record("gen_ai.request.model", entry.model.as_str());
                    span.record("rg.served_from", ServedFrom::ResponseCache.as_str());
                    span.record("rg.attempts", 0u8);
                    span.record("rg.cost_usd_micros", 0u64);
                    span.record("rg.finish_reason", "complete");
                    return Ok(ModelResponse {
                        output: entry.output,
                        usage: Usage::default(),
                        latency_ms: 0,
                        provider: first.provider.clone(),
                        model: entry.model,
                        cost_usd_micros: Some(0),
                        finish_reason: FinishReason::Complete,
                        request_hash: hash,
                        route,
                        attempts: 0,
                        served_from: ServedFrom::ResponseCache,
                        usage_original: Some(entry.usage),
                    });
                }
                Ok(None) => self.metrics.cache_miss(req.task.as_str()),
                Err(e) => {
                    self.metrics.cache_miss(req.task.as_str());
                    tracing::warn!(error = %e, "response cache read failed; treating as a miss");
                }
            }
        }

        let started = Instant::now();
        let mut attempts_total: u8 = 0;
        let mut last_err: Option<GatewayError> = None;
        let candidates = route.candidates.clone();
        let count = candidates.len();
        for (rank, candidate) in candidates.iter().enumerate() {
            // Defence in depth: the privacy filter is asserted again right before any send.
            if !self.router.permits(&candidate.provider, req.privacy) {
                last_err = Some(GatewayError::NoEligibleProvider {
                    tier: req.tier,
                    privacy: req.privacy,
                    reason: format!(
                        "provider {} is not permitted for this privacy class",
                        candidate.provider
                    ),
                });
                continue;
            }
            let Some(adapter) = self.adapters.get(&candidate.provider) else {
                last_err = Some(GatewayError::Permanent {
                    kind: PermanentKind::Unknown,
                    provider: Some(candidate.provider.clone()),
                    detail: "provider not registered".into(),
                });
                continue;
            };
            precheck_cost(
                &req,
                est_input,
                &candidate.provider,
                &candidate.model,
                self.pricer.as_deref(),
            )?;
            let limit_req = self
                .router
                .limits(&candidate.provider, &candidate.model)
                .map(|limits| LimitRequest {
                    provider: candidate.provider.clone(),
                    model: candidate.model.clone(),
                    limits,
                    est_input,
                    max_output: req.max_output_tokens,
                    deadline: req.budget.deadline,
                });
            let leg = Leg {
                req: &req,
                hash: &hash,
                candidate,
                adapter: adapter.as_ref(),
                limit: limit_req.as_ref(),
            };
            let outcome = self.run_leg(leg, 0, req.budget.max_attempts, &cancel).await;
            attempts_total = attempts_total.saturating_add(outcome.attempts);
            let resp = match outcome.result {
                Ok(resp) => resp,
                Err(e) => {
                    let more = rank + 1 < count;
                    if e.fallback_eligible() && more {
                        tracing::warn!(
                            from = %candidate.provider,
                            reason = e.class(),
                            "falling back to next candidate"
                        );
                        self.metrics
                            .fallback(&Self::labels(&req, candidate), e.class());
                        route.attempted.push(AttemptedCandidate {
                            provider: candidate.provider.clone(),
                            model: candidate.model.clone(),
                            error_class: e.class().to_owned(),
                        });
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            };
            let resp = self
                .validate_and_repair(
                    leg,
                    resp,
                    outcome.attempts,
                    &mut attempts_total,
                    &cancel,
                    span,
                )
                .await?;

            let cost = self.cost_of(candidate, &resp.model, &resp.usage);
            if let (Some(ttl), Some(cache)) = (cache_ttl, &self.cache) {
                if resp.finish_reason == FinishReason::Complete {
                    let key = cache_key_for(&req, &hash, candidate);
                    let entry = CacheEntry {
                        request_hash: hash.0.clone(),
                        provider: candidate.provider.0.clone(),
                        model: candidate.model.clone(),
                        prompt_version: req.input.system.prompt_version.clone(),
                        schema_hash: req.output_schema.as_ref().map(|s| s.hash.clone()),
                        output: resp.output.clone(),
                        usage: resp.usage,
                    };
                    if let Err(e) = cache
                        .put(req.tenant.organization_id, &key, entry, ttl)
                        .await
                    {
                        tracing::warn!(error = %e, "response cache write failed");
                    }
                }
            }
            let u = resp.usage;
            self.metrics.usage(&Self::labels(&req, candidate), &u, cost);
            span.record("gen_ai.system", candidate.provider.as_str());
            span.record("gen_ai.request.model", candidate.model.as_str());
            span.record(
                "gen_ai.usage.input_tokens",
                u.input_uncached + u.cache_read + u.cache_write,
            );
            span.record("gen_ai.usage.output_tokens", u.output);
            span.record("rg.attempts", attempts_total);
            span.record("rg.served_from", resp.served_from.as_str());
            span.record("rg.finish_reason", finish_str(&resp.finish_reason));
            if let Some(c) = cost {
                span.record("rg.cost_usd_micros", c);
            }
            return Ok(ModelResponse {
                output: resp.output,
                usage: resp.usage,
                latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
                provider: candidate.provider.clone(),
                model: resp.model,
                cost_usd_micros: cost,
                finish_reason: resp.finish_reason,
                request_hash: hash,
                route,
                attempts: attempts_total,
                served_from: resp.served_from,
                usage_original: None,
            });
        }
        Err(last_err.unwrap_or(GatewayError::NoEligibleProvider {
            tier: req.tier,
            privacy: req.privacy,
            reason: "router returned no candidates".into(),
        }))
    }
}

fn cache_key_for(req: &ModelRequest, hash: &RequestHash, c: &RouteCandidate) -> String {
    cache_key(
        req.tenant.organization_id,
        hash.as_str(),
        c.provider.as_str(),
        &c.model,
        req.output_schema.as_ref().map(|s| s.hash.as_str()),
    )
}

#[async_trait]
impl ModelGateway for Gateway {
    async fn call(
        &self,
        req: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelResponse, GatewayError> {
        // Span `model_request` (target-architecture §8). Prompt, output and section contents are
        // never recorded; only names, counts, hashes and correlation ids.
        let span = tracing::info_span!(
            "model_request",
            "gen_ai.system" = Empty,
            "gen_ai.request.model" = Empty,
            "gen_ai.operation.name" = "chat",
            "gen_ai.usage.input_tokens" = Empty,
            "gen_ai.usage.output_tokens" = Empty,
            "rg.task" = req.task.as_str(),
            "rg.tier" = req.tier.as_str(),
            "rg.effective_tier" = Empty,
            "rg.request_hash" = Empty,
            "rg.attempts" = Empty,
            "rg.served_from" = Empty,
            "rg.finish_reason" = Empty,
            "rg.cost_usd_micros" = Empty,
            "rg.redactions" = Empty,
            "rg.repair" = Empty,
            "rg.outcome" = Empty,
            review_run_id = Empty,
            organization_id = %req.tenant.organization_id,
            repository_id = %req.tenant.repository_id,
            reviewer_type = Empty,
        );
        if let Some(id) = req.trace.review_run_id {
            span.record("review_run_id", tracing::field::display(id));
        }
        if let Some(r) = req.trace.reviewer_type {
            span.record("reviewer_type", r.as_str());
        }
        let task = req.task.as_str();
        let tier = req.tier.as_str();
        let started = Instant::now();
        let result = self
            .call_inner(req, cancel, &span)
            .instrument(span.clone())
            .await;
        let outcome = match &result {
            Ok(_) => "ok",
            Err(e) => e.class(),
        };
        span.record("rg.outcome", outcome);
        let (provider, model) = match &result {
            Ok(r) => (r.provider.0.clone(), r.model.clone()),
            Err(_) => ("none".to_owned(), "none".to_owned()),
        };
        self.metrics.request(
            &CallLabels {
                provider,
                model,
                task,
                tier,
            },
            outcome,
            started.elapsed().as_secs_f64(),
        );
        result
    }
}
