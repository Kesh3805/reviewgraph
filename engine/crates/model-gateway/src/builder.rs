//! Gateway core and its builder (GW-001). Later tasks wire their parts in; here every part has
//! a minimal implementation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::adapter::{ProviderAdapter, ProviderRequest};
use crate::error::{BudgetKind, Error, GatewayError, PermanentKind};
use crate::request_hash::request_hash;
use crate::retry::{retry, RetryPolicy};
use crate::types::{
    AttemptedCandidate, ModelRequest, ModelResponse, ModelTier, PrivacyClass, ProviderId, RiskBand,
    RouteCandidate, RouteDecision,
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
        Ok(Gateway {
            adapters,
            router,
            retry: self.retry.unwrap_or_default(),
        })
    }
}

/// The gateway core. Holds no locks across `.await`.
pub struct Gateway {
    adapters: HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
    router: Arc<dyn RouteSource>,
    retry: RetryPolicy,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway")
            .field("providers", &self.adapters.keys().collect::<Vec<_>>())
            .finish()
    }
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
            est_input_tokens: estimate_input_tokens(&req),
            max_output_tokens: req.max_output_tokens,
            remaining_budget_fraction: req.budget.remaining_fraction,
            schema_strict_ok: strict_ok,
        };
        let route = self.router.route(&registered, &query)?;
        let mut route = route;

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
            let req_ref = &req;
            let hash_ref = &hash;
            let outcome = retry(&self.retry, &req.budget, &cancel, |_attempt| async move {
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
                adapter.send(&provider_req).await
            })
            .await;
            attempts_total = attempts_total.saturating_add(outcome.attempts);
            match outcome.result {
                Ok(resp) => {
                    return Ok(ModelResponse {
                        output: resp.output,
                        usage: resp.usage,
                        latency_ms: u32::try_from(started.elapsed().as_millis())
                            .unwrap_or(u32::MAX),
                        provider: candidate.provider.clone(),
                        model: resp.model,
                        cost_usd_micros: None,
                        finish_reason: resp.finish_reason,
                        request_hash: hash,
                        route,
                        attempts: attempts_total,
                        served_from: resp.served_from,
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
