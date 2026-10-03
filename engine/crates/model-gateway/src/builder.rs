//! Gateway core and its builder (GW-001). Later tasks wire their parts in; here every part has
//! a minimal implementation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use uuid::Uuid;

use crate::accounting::{LedgerRecord, LedgerSink, NoLedger, PriceTable};
use crate::adapter::{ProviderAdapter, ProviderRequest};
use crate::budget::{precheck_cost, precheck_tokens, WorstCasePricer};
use crate::cache::{cache_key, CacheEntry, ResponseCache};
use crate::error::{BudgetKind, Error, GatewayError, PermanentKind};
use crate::ratelimit::{LimitRequest, ModelLimits, NoLimit, RateLimiter};
use crate::request_hash::request_hash;
use crate::retry::{retry, RetryPolicy};
use crate::types::{
    AttemptedCandidate, CachePolicy, FinishReason, ModelRequest, ModelResponse, ModelTier,
    PrivacyClass, ProviderId, RequestHash, RiskBand, RouteCandidate, RouteDecision, ServedFrom,
    Usage,
};

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

/// Composes adapters, router and the other gateway parts.
#[derive(Default)]
pub struct GatewayBuilder {
    adapters: Vec<Arc<dyn ProviderAdapter>>,
    router: Option<Arc<dyn RouteSource>>,
    retry: Option<RetryPolicy>,
    limiter: Option<Arc<dyn RateLimiter>>,
    pricer: Option<Arc<dyn WorstCasePricer>>,
    prices: Option<Arc<PriceTable>>,
    ledger: Option<Arc<dyn LedgerSink>>,
    cache: Option<Arc<dyn ResponseCache>>,
}

impl std::fmt::Debug for GatewayBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayBuilder")
            .field("adapters", &self.adapters.len())
            .finish()
    }
}

impl GatewayBuilder {
    pub fn new() -> Self {
        Self::default()
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
        let now = Instant::now();
        if req.budget.deadline <= now {
            return Err(GatewayError::BudgetExceeded {
                kind: BudgetKind::Deadline,
            });
        }
        if cancel.is_cancelled() {
            return Err(GatewayError::Cancelled);
        }
        let hash = request_hash(&req);
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

        // Response cache: after routing (provider and model are known), before rate limiting.
        let cache_ttl = self.cache_ttl(&req);
        if let (Some(cache), Some(first), Some(_)) =
            (&self.cache, route.candidates.first(), cache_ttl)
        {
            let key = cache_key_for(&req, &hash, first);
            match cache.get(req.tenant.organization_id, &key).await {
                Ok(Some(entry)) => {
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
                Ok(None) => {}
                Err(e) => {
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
            let this = self;
            let req_ref = &req;
            let hash_ref = &hash;
            let limit_ref = limit_req.as_ref();
            let cancel_ref = &cancel;
            let outcome = retry(&self.retry, &req.budget, &cancel, |attempt| async move {
                let reservation = match limit_ref {
                    Some(lr) => Some(this.limiter.acquire(lr, cancel_ref).await?),
                    None => None,
                };
                let remaining = req_ref
                    .budget
                    .deadline
                    .saturating_duration_since(Instant::now());
                let provider_req = ProviderRequest {
                    request: req_ref,
                    candidate,
                    request_hash: hash_ref,
                    timeout: remaining.min(MAX_ATTEMPT_TIMEOUT),
                };
                let attempt_started = Instant::now();
                let result = adapter.send(&provider_req).await;
                let latency_ms =
                    u32::try_from(attempt_started.elapsed().as_millis()).unwrap_or(u32::MAX);
                match result {
                    Ok(resp) => {
                        let u = resp.usage;
                        if let Some(r) = &reservation {
                            let input = u.input_uncached + u.cache_write + u.cache_read;
                            this.limiter.reconcile(r, input, u.output).await;
                        }
                        this.record(AttemptFacts {
                            req: req_ref,
                            hash: hash_ref,
                            candidate,
                            attempt,
                            served_from: resp.served_from,
                            outcome: "ok".into(),
                            usage: u,
                            cost: this.cost_of(candidate, &resp.model, &u),
                            latency_ms,
                        });
                        Ok(resp)
                    }
                    Err(e) => {
                        this.record(AttemptFacts {
                            req: req_ref,
                            hash: hash_ref,
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
            })
            .await;
            attempts_total = attempts_total.saturating_add(outcome.attempts);
            match outcome.result {
                Ok(resp) => {
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
                    return Ok(ModelResponse {
                        output: resp.output,
                        usage: resp.usage,
                        latency_ms: u32::try_from(started.elapsed().as_millis())
                            .unwrap_or(u32::MAX),
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
                Err(e) => {
                    let more = rank + 1 < count;
                    if e.fallback_eligible() && more {
                        tracing::warn!(
                            from = %candidate.provider,
                            reason = e.class(),
                            "falling back to next candidate"
                        );
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
            }
        }
        Err(last_err.unwrap_or(GatewayError::NoEligibleProvider {
            tier: req.tier,
            privacy: req.privacy,
            reason: "router returned no candidates".into(),
        }))
    }
}
