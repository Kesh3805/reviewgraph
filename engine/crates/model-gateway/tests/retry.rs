#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use model_gateway::classify::{
    classify_http, classify_transport, parse_retry_after, TransportFailure,
};
use model_gateway::{
    retry, BudgetKind, CallBudget, GatewayError, JitterRng, ModelTier, PermanentKind, PrivacyClass,
    ProviderId, RateScope, RetryPolicy, SchemaErrorSummary, TransientKind,
};
use proptest::prelude::*;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

fn p() -> ProviderId {
    ProviderId::new("anthropic")
}

fn http(status: u16, body: &str) -> GatewayError {
    classify_http(&p(), status, body, None, SystemTime::now())
}

fn perm_kind(e: &GatewayError) -> PermanentKind {
    match e {
        GatewayError::Permanent { kind, .. } => *kind,
        other => panic!("expected permanent, got {other:?}"),
    }
}

#[test]
fn quota_429_is_permanent_not_rate_limited() {
    let e = http(
        429,
        r#"{"error":{"type":"insufficient_quota","message":"You exceeded your current quota"}}"#,
    );
    assert_eq!(perm_kind(&e), PermanentKind::QuotaExhausted);
    assert!(!e.is_retryable());
    let e = http(
        400,
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low"}}"#,
    );
    assert_eq!(perm_kind(&e), PermanentKind::QuotaExhausted);
}

#[test]
fn anthropic_529_is_transient_overloaded() {
    let e = http(
        529,
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
    );
    assert!(matches!(
        e,
        GatewayError::Transient {
            kind: TransientKind::Overloaded,
            ..
        }
    ));
}

#[test]
fn plain_429_is_rate_limited_with_retry_after() {
    let e = classify_http(&p(), 429, "slow down", Some("7"), SystemTime::now());
    match e {
        GatewayError::RateLimited {
            retry_after, scope, ..
        } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)));
            assert_eq!(scope, RateScope::Provider);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn retry_after_seconds_and_http_date_parsed() {
    let now = httpdate::parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT").unwrap();
    assert_eq!(
        parse_retry_after("120", now),
        Some(Duration::from_secs(120))
    );
    assert_eq!(
        parse_retry_after("Wed, 21 Oct 2015 07:28:30 GMT", now),
        Some(Duration::from_secs(30))
    );
    // A date in the past means "retry now".
    assert_eq!(
        parse_retry_after("Wed, 21 Oct 2015 07:27:00 GMT", now),
        Some(Duration::ZERO)
    );
    assert_eq!(parse_retry_after("garbage", now), None);
}

#[test]
fn permanent_rules_win_over_transient_words() {
    // A 503 whose body talks about billing is not transient.
    let e = http(503, "service unavailable: billing issue");
    assert_eq!(perm_kind(&e), PermanentKind::QuotaExhausted);
    // A 500 that says the model was not found is permanent too.
    let e = http(
        500,
        r#"{"error":{"message":"The model `x` was not found"}}"#,
    );
    assert_eq!(perm_kind(&e), PermanentKind::ModelNotFound);
}

#[test]
fn unknown_errors_are_not_retried() {
    let e = http(418, "teapot");
    assert_eq!(perm_kind(&e), PermanentKind::Unknown);
    assert!(!e.is_retryable());
    assert!(!http(501, "not implemented").is_retryable());
}

#[test]
fn connection_reset_is_transient() {
    for (f, k) in [
        (TransportFailure::Connect, TransientKind::Connect),
        (TransportFailure::Timeout, TransientKind::Timeout),
        (TransportFailure::Reset, TransientKind::Reset),
        (TransportFailure::BodyRead, TransientKind::TruncatedBody),
    ] {
        match classify_transport(&p(), f) {
            GatewayError::Transient { kind, .. } => assert_eq!(kind, k),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn classification_table_rows() {
    let invalid = r#"{"error":{"type":"invalid_request_error","message":"bad field"}}"#;
    assert_eq!(
        perm_kind(&http(400, invalid)),
        PermanentKind::InvalidRequest
    );
    let unsupported = r#"{"error":{"type":"invalid_request_error","message":"Unsupported parameter: 'temperature'"}}"#;
    assert_eq!(
        perm_kind(&http(400, unsupported)),
        PermanentKind::UnsupportedParameter
    );
    assert_eq!(perm_kind(&http(401, "no")), PermanentKind::Auth);
    assert_eq!(perm_kind(&http(403, "no")), PermanentKind::Forbidden);
    assert_eq!(perm_kind(&http(404, "no")), PermanentKind::ModelNotFound);
    assert_eq!(perm_kind(&http(413, "big")), PermanentKind::ContextTooLarge);
    let too_long = r#"{"error":{"type":"invalid_request_error","message":"prompt is too long: 250000 tokens"}}"#;
    assert_eq!(
        perm_kind(&http(400, too_long)),
        PermanentKind::ContextTooLarge
    );
    let ctx = r#"{"error":{"code":"context_length_exceeded","message":"x"}}"#;
    assert_eq!(perm_kind(&http(400, ctx)), PermanentKind::ContextTooLarge);
    for s in [408u16, 500, 502, 503, 504] {
        assert!(matches!(
            http(s, "oops"),
            GatewayError::Transient {
                kind: TransientKind::ServerError(code),
                ..
            } if code == s
        ));
    }
}

#[test]
fn detail_is_truncated_and_scrubbed() {
    let long = format!("{} ghp_abcdefghijklmnopqrstuvwxyz0123", "x".repeat(2000));
    match http(401, &long) {
        GatewayError::Permanent { detail, .. } => {
            assert!(detail.chars().count() <= 500);
            assert!(!detail.contains("ghp_abc"));
        }
        other => panic!("{other:?}"),
    }
    let short = "denied ghp_abcdefghijklmnopqrstuvwxyz0123";
    match http(401, short) {
        GatewayError::Permanent { detail, .. } => assert!(!detail.contains("ghp_abc"), "{detail}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn fallback_eligibility_matrix() {
    let perm = |kind| GatewayError::Permanent {
        kind,
        provider: None,
        detail: String::new(),
    };
    use PermanentKind::*;
    for k in [
        Auth,
        Forbidden,
        ModelNotFound,
        QuotaExhausted,
        UnsupportedParameter,
    ] {
        assert!(perm(k).fallback_eligible(), "{k:?}");
    }
    for k in [
        InvalidRequest,
        ContextTooLarge,
        Refusal,
        ReplayMiss,
        Unknown,
    ] {
        assert!(!perm(k).fallback_eligible(), "{k:?}");
    }
    assert!(GatewayError::Transient {
        kind: TransientKind::Timeout,
        provider: None
    }
    .fallback_eligible());
    assert!(GatewayError::RateLimited {
        retry_after: None,
        provider: p(),
        scope: RateScope::Provider
    }
    .fallback_eligible());
    assert!(!GatewayError::BudgetExceeded {
        kind: BudgetKind::Cost
    }
    .fallback_eligible());
    assert!(!GatewayError::Cancelled.fallback_eligible());
    assert!(!GatewayError::NoEligibleProvider {
        tier: ModelTier::Verifier,
        privacy: PrivacyClass::NoExternal,
        reason: String::new()
    }
    .fallback_eligible());
    assert!(!GatewayError::SchemaViolation {
        errors: Vec::<SchemaErrorSummary>::new(),
        repaired: false
    }
    .fallback_eligible());
}

struct MaxRng;
impl JitterRng for MaxRng {
    fn uniform(&self, upper: Duration) -> Duration {
        upper
    }
}

fn transient() -> GatewayError {
    GatewayError::Transient {
        kind: TransientKind::Timeout,
        provider: Some(p()),
    }
}

#[test]
fn backoff_is_full_jitter_bounded_by_cap() {
    let policy = RetryPolicy::default();
    for attempt in 1..=40 {
        let bound = policy.jitter_bound(attempt);
        assert!(bound <= Duration::from_secs(20));
        for _ in 0..50 {
            assert!(policy.backoff(attempt, &transient()) <= bound);
        }
    }
    assert_eq!(policy.jitter_bound(1), Duration::from_millis(500));
    assert_eq!(policy.jitter_bound(2), Duration::from_millis(1000));
    // Rate-limit sleeps honour Retry-After even beyond the jitter bound.
    let rl = GatewayError::RateLimited {
        retry_after: Some(Duration::from_secs(30)),
        provider: p(),
        scope: RateScope::Provider,
    };
    assert!(policy.backoff(1, &rl) >= Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn retry_stops_before_deadline() {
    let policy = RetryPolicy::default().with_rng(Arc::new(MaxRng));
    let budget = CallBudget::within(Duration::from_secs(1));
    let calls = AtomicU32::new(0);
    let out = retry(&policy, &budget, &CancellationToken::new(), |_| async {
        calls.fetch_add(1, Ordering::SeqCst);
        Err::<(), _>(transient())
    })
    .await;
    assert!(matches!(
        out.result,
        Err(GatewayError::BudgetExceeded {
            kind: BudgetKind::Deadline
        })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn transient_then_success_is_retried() {
    let policy = RetryPolicy::default().with_rng(Arc::new(MaxRng));
    let budget = CallBudget::within(Duration::from_secs(60));
    let calls = AtomicU32::new(0);
    let out = retry(&policy, &budget, &CancellationToken::new(), |attempt| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n < 2 {
                Err(transient())
            } else {
                Ok(attempt)
            }
        }
    })
    .await;
    assert_eq!(out.result.unwrap(), 3);
    assert_eq!(out.attempts, 3);
}

#[tokio::test(start_paused = true)]
async fn attempts_are_capped_at_three() {
    let policy = RetryPolicy::default().with_rng(Arc::new(MaxRng));
    let mut budget = CallBudget::within(Duration::from_secs(600));
    budget.max_attempts = 10;
    let calls = AtomicU32::new(0);
    let out = retry(&policy, &budget, &CancellationToken::new(), |_| async {
        calls.fetch_add(1, Ordering::SeqCst);
        Err::<(), _>(transient())
    })
    .await;
    assert!(out.result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn schema_violation_not_retried_by_retry_loop() {
    let policy = RetryPolicy::default();
    let budget = CallBudget::within(Duration::from_secs(60));
    let calls = AtomicU32::new(0);
    let out = retry(&policy, &budget, &CancellationToken::new(), |_| async {
        calls.fetch_add(1, Ordering::SeqCst);
        Err::<(), _>(GatewayError::SchemaViolation {
            errors: vec![],
            repaired: false,
        })
    })
    .await;
    assert!(matches!(
        out.result,
        Err(GatewayError::SchemaViolation { .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn cancel_during_backoff_returns_cancelled() {
    let policy = RetryPolicy::default().with_rng(Arc::new(MaxRng));
    let budget = CallBudget::within(Duration::from_secs(600));
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        c2.cancel();
    });
    let started = Instant::now();
    let out = retry(&policy, &budget, &cancel, |_| async {
        Err::<(), _>(transient())
    })
    .await;
    assert!(matches!(out.result, Err(GatewayError::Cancelled)));
    assert!(started.elapsed() < Duration::from_millis(500));
}

proptest! {
    #[test]
    fn total_sleep_never_exceeds_deadline(deadline_ms in 100u64..30_000, retry_after_ms in 0u64..10_000, rate_limited in any::<bool>()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap();
        rt.block_on(async {
            let policy = RetryPolicy::default();
            let budget = CallBudget::within(Duration::from_millis(deadline_ms));
            let start = Instant::now();
            let _ = retry(&policy, &budget, &CancellationToken::new(), |_| async move {
                Err::<(), _>(if rate_limited {
                    GatewayError::RateLimited {
                        retry_after: Some(Duration::from_millis(retry_after_ms)),
                        provider: ProviderId::new("anthropic"),
                        scope: RateScope::Provider,
                    }
                } else {
                    GatewayError::Transient { kind: TransientKind::Timeout, provider: None }
                })
            })
            .await;
            prop_assert!(start.elapsed() <= Duration::from_millis(deadline_ms));
            Ok(())
        })?;
    }
}
