#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use model_gateway::ratelimit::local::LocalBuckets;
use model_gateway::ratelimit::{
    BucketSpec, BucketStore, FallbackStore, LimitRequest, ModelLimits, RateLimiter, StoreError,
    Take, TokenBucketLimiter,
};
use model_gateway::{
    BudgetKind, CallBudget, FinishReason, GatewayBuilder, GatewayError, ModelGateway, ModelOutput,
    ModelTier, ProviderAdapter, ProviderId, ProviderRequest, ProviderResponse, RateScope,
    RouteCandidate, ServedFrom, StaticRouter, Usage, WorstCasePricer,
};
use serde_json::json;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use common::request;

fn limit_req(
    limits: ModelLimits,
    est_input: u32,
    max_output: u32,
    within: Duration,
) -> LimitRequest {
    LimitRequest {
        provider: ProviderId::new("anthropic"),
        model: "m".into(),
        limits,
        est_input,
        max_output,
        deadline: Instant::now() + within,
    }
}

const BIG: ModelLimits = ModelLimits {
    rpm: 60,
    input_tpm: 1_000_000,
    output_tpm: 1_000_000,
};

fn limiter() -> (TokenBucketLimiter, Arc<LocalBuckets>) {
    let store = Arc::new(LocalBuckets::new());
    (TokenBucketLimiter::new(store.clone()), store)
}

#[tokio::test(start_paused = true)]
async fn bucket_allows_within_capacity() {
    let store = LocalBuckets::new();
    let spec = BucketSpec::per_minute(10);
    for _ in 0..10 {
        assert_eq!(store.take("k", spec, 1.0).await.unwrap(), Take::Granted);
    }
    assert!(matches!(
        store.take("k", spec, 1.0).await.unwrap(),
        Take::Denied { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn bucket_denies_and_reports_wait() {
    let store = LocalBuckets::new();
    let spec = BucketSpec::per_minute(60); // 1 token per second
    assert_eq!(store.take("k", spec, 60.0).await.unwrap(), Take::Granted);
    match store.take("k", spec, 1.0).await.unwrap() {
        Take::Denied { wait } => assert!(
            wait >= Duration::from_millis(990) && wait <= Duration::from_millis(1010),
            "{wait:?}"
        ),
        Take::Granted => panic!("must be denied"),
    }
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(store.take("k", spec, 1.0).await.unwrap(), Take::Granted);
}

#[tokio::test(start_paused = true)]
async fn partial_acquire_refunds() {
    let (lim, store) = limiter();
    // Input bucket (1000/min) cannot cover 900 twice: the second acquire fails on in_tok after
    // taking one request token, which must be given back.
    let limits = ModelLimits {
        rpm: 60,
        input_tpm: 1000,
        output_tpm: 1_000_000,
    };
    let cancel = CancellationToken::new();
    lim.acquire(
        &limit_req(limits, 900, 10, Duration::from_secs(60)),
        &cancel,
    )
    .await
    .unwrap();
    let err = lim
        .acquire(
            &limit_req(limits, 900, 10, Duration::from_millis(10)),
            &cancel,
        )
        .await
        .expect_err("input bucket exhausted");
    assert!(matches!(
        err,
        GatewayError::RateLimited {
            scope: RateScope::InputTokens,
            ..
        }
    ));
    // 60 request tokens: 1 used by the first call; the failed attempt must have refunded its own.
    let spec = BucketSpec::per_minute(60);
    let key = "rg:rl:anthropic:m:req";
    for i in 0..59 {
        assert_eq!(
            store.take(key, spec, 1.0).await.unwrap(),
            Take::Granted,
            "token {i}"
        );
    }
    assert!(matches!(
        store.take(key, spec, 1.0).await.unwrap(),
        Take::Denied { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn reconcile_refunds_unused_output() {
    let (lim, store) = limiter();
    let limits = ModelLimits {
        rpm: 60,
        input_tpm: 1_000_000,
        output_tpm: 1000,
    };
    let cancel = CancellationToken::new();
    let r = lim
        .acquire(
            &limit_req(limits, 10, 1000, Duration::from_secs(60)),
            &cancel,
        )
        .await
        .unwrap();
    // The whole output bucket is reserved; only 100 tokens were actually produced.
    lim.reconcile(&r, 10, 100).await;
    let spec = BucketSpec::per_minute(1000);
    assert_eq!(
        store
            .take("rg:rl:anthropic:m:out_tok", spec, 900.0)
            .await
            .unwrap(),
        Take::Granted
    );
    assert!(matches!(
        store
            .take("rg:rl:anthropic:m:out_tok", spec, 100.0)
            .await
            .unwrap(),
        Take::Denied { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn reconcile_adjusts_input_both_ways() {
    let (lim, store) = limiter();
    let limits = ModelLimits {
        rpm: 60,
        input_tpm: 1000,
        output_tpm: 1_000_000,
    };
    let cancel = CancellationToken::new();
    let r = lim
        .acquire(
            &limit_req(limits, 500, 10, Duration::from_secs(60)),
            &cancel,
        )
        .await
        .unwrap();
    lim.reconcile(&r, 200, 10).await; // overestimated by 300: 800 left
    let spec = BucketSpec::per_minute(1000);
    assert_eq!(
        store
            .take("rg:rl:anthropic:m:in_tok", spec, 800.0)
            .await
            .unwrap(),
        Take::Granted
    );
}

#[tokio::test(start_paused = true)]
async fn oversized_cost_clamped_to_capacity() {
    let (lim, _) = limiter();
    let limits = ModelLimits {
        rpm: 60,
        input_tpm: 1000,
        output_tpm: 1000,
    };
    // 300k estimated input tokens against a 1000/min bucket: must not deadlock.
    let r = lim
        .acquire(
            &limit_req(limits, 300_000, 500, Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .await;
    assert!(r.is_ok());
}

#[tokio::test(start_paused = true)]
async fn waits_then_succeeds_within_deadline() {
    let (lim, _) = limiter();
    let limits = ModelLimits {
        rpm: 60,
        input_tpm: 1_000_000,
        output_tpm: 1_000_000,
    };
    let cancel = CancellationToken::new();
    for _ in 0..60 {
        lim.acquire(&limit_req(limits, 1, 1, Duration::from_secs(600)), &cancel)
            .await
            .unwrap();
    }
    let started = Instant::now();
    lim.acquire(&limit_req(limits, 1, 1, Duration::from_secs(30)), &cancel)
        .await
        .expect("waits for refill");
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[tokio::test(start_paused = true)]
async fn wait_beyond_deadline_returns_rate_limited() {
    let (lim, _) = limiter();
    let cancel = CancellationToken::new();
    for _ in 0..60 {
        lim.acquire(&limit_req(BIG, 1, 1, Duration::from_secs(600)), &cancel)
            .await
            .unwrap();
    }
    let err = lim
        .acquire(&limit_req(BIG, 1, 1, Duration::from_millis(200)), &cancel)
        .await
        .expect_err("deadline first");
    match &err {
        GatewayError::RateLimited {
            retry_after, scope, ..
        } => {
            assert!(retry_after.expect("wait") >= Duration::from_millis(900));
            assert_eq!(*scope, RateScope::Requests);
        }
        other => panic!("{other:?}"),
    }
    assert!(err.fallback_eligible());
    assert!(
        !err.is_retryable(),
        "limiter denials are not retried by the retry loop"
    );
}

#[tokio::test(start_paused = true)]
async fn cancel_while_waiting() {
    let (lim, _) = limiter();
    let cancel = CancellationToken::new();
    for _ in 0..60 {
        lim.acquire(&limit_req(BIG, 1, 1, Duration::from_secs(600)), &cancel)
            .await
            .unwrap();
    }
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        c2.cancel();
    });
    let err = lim
        .acquire(&limit_req(BIG, 1, 1, Duration::from_secs(600)), &cancel)
        .await
        .expect_err("cancelled");
    assert!(matches!(err, GatewayError::Cancelled));
}

struct DownStore(AtomicUsize);

#[async_trait]
impl BucketStore for DownStore {
    async fn take(&self, _k: &str, _s: BucketSpec, _c: f64) -> Result<Take, StoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(StoreError("connection refused".into()))
    }

    async fn refund(&self, _k: &str, _s: BucketSpec, _a: f64) -> Result<(), StoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(StoreError("connection refused".into()))
    }
}

#[tokio::test(start_paused = true)]
async fn redis_down_falls_back_to_local() {
    let down = Arc::new(DownStore(AtomicUsize::new(0)));
    let store = Arc::new(FallbackStore::new(down.clone(), 4));
    let lim = TokenBucketLimiter::new(store.clone());
    let limits = ModelLimits {
        rpm: 40,
        input_tpm: 1_000_000,
        output_tpm: 1_000_000,
    };
    let cancel = CancellationToken::new();
    // Local capacity is 40 / 4 = 10 requests per minute.
    for _ in 0..10 {
        lim.acquire(&limit_req(limits, 1, 1, Duration::from_millis(1)), &cancel)
            .await
            .expect("local bucket");
    }
    let err = lim
        .acquire(&limit_req(limits, 1, 1, Duration::from_millis(1)), &cancel)
        .await
        .expect_err("local bucket exhausted");
    assert!(matches!(err, GatewayError::RateLimited { .. }));
    assert!(store.degraded_total() >= 10);
    // The circuit stays open: Redis was probed once, not on every call.
    assert_eq!(down.0.load(Ordering::SeqCst), 1);
}

// ---- budget pre-checks through the gateway ----

struct Counting(Arc<AtomicUsize>);

#[async_trait]
impl ProviderAdapter for Counting {
    fn provider(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    async fn send(&self, _r: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ProviderResponse {
            output: ModelOutput::Json(json!({"ok": true})),
            usage: Usage {
                input_uncached: 10,
                cache_write: 0,
                cache_read: 0,
                output: 5,
                reasoning: 0,
            },
            finish_reason: FinishReason::Complete,
            model: "m".into(),
            provider_request_id: None,
            tool_use_id: None,
            served_from: ServedFrom::Live,
        })
    }
}

struct FixedPrice(u64);

impl WorstCasePricer for FixedPrice {
    fn worst_case_micros(&self, _p: &ProviderId, _m: &str, _i: u32, _o: u32) -> Option<u64> {
        Some(self.0)
    }
}

fn gateway(calls: Arc<AtomicUsize>, pricer: Option<u64>) -> model_gateway::Gateway {
    let router = StaticRouter::new().with_route(
        ModelTier::ReviewReasoner,
        vec![RouteCandidate {
            provider: ProviderId::new("anthropic"),
            model: "m".into(),
            max_context: 200_000,
            supports_reasoning: false,
        }],
    );
    let mut b = GatewayBuilder::new()
        .adapter(Arc::new(Counting(calls)))
        .router(Arc::new(router));
    if let Some(p) = pricer {
        b = b.pricer(Arc::new(FixedPrice(p)));
    }
    b.build().expect("build")
}

#[tokio::test]
async fn budget_precheck_input_tokens() {
    let calls = Arc::new(AtomicUsize::new(0));
    let gw = gateway(calls.clone(), None);
    let mut req = request();
    req.budget = CallBudget {
        max_input_tokens: 5,
        ..CallBudget::within(Duration::from_secs(60))
    };
    let err = gw
        .call(req, CancellationToken::new())
        .await
        .expect_err("too big");
    assert!(matches!(
        err,
        GatewayError::BudgetExceeded {
            kind: BudgetKind::InputTokens
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn budget_precheck_output_tokens() {
    let calls = Arc::new(AtomicUsize::new(0));
    let gw = gateway(calls.clone(), None);
    let mut req = request();
    req.max_output_tokens = 20_000;
    let err = gw
        .call(req, CancellationToken::new())
        .await
        .expect_err("too big");
    assert!(matches!(
        err,
        GatewayError::BudgetExceeded {
            kind: BudgetKind::OutputTokens
        }
    ));
}

#[tokio::test]
async fn budget_precheck_cost() {
    let calls = Arc::new(AtomicUsize::new(0));
    let gw = gateway(calls.clone(), Some(5_000));
    let mut req = request();
    req.budget.max_cost_usd_micros = Some(1_000);
    let err = gw
        .call(req, CancellationToken::new())
        .await
        .expect_err("too expensive");
    assert!(matches!(
        err,
        GatewayError::BudgetExceeded {
            kind: BudgetKind::Cost
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let mut ok = request();
    ok.budget.max_cost_usd_micros = Some(10_000);
    gw.call(ok, CancellationToken::new())
        .await
        .expect("within budget");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
