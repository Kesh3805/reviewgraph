#![cfg(feature = "integration")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! Needs Redis: `REDIS_URL` (default `redis://host.docker.internal:26379`).
//! Run with `--features integration`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use model_gateway::ratelimit::redis_bucket::RedisBuckets;
use model_gateway::ratelimit::{
    BucketSpec, BucketStore, FallbackStore, LimitRequest, ModelLimits, RateLimiter, Take,
    TokenBucketLimiter,
};
use model_gateway::ProviderId;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

fn url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://host.docker.internal:26379".into())
}

/// A key prefix unique to this test run, so tests never see each other's buckets.
fn key(name: &str) -> String {
    format!("rg:rl:test:{}:{name}", uuid::Uuid::new_v4())
}

async fn store() -> RedisBuckets {
    RedisBuckets::connect(&url())
        .await
        .expect("redis reachable")
}

#[tokio::test]
async fn bucket_allows_within_capacity() {
    let s = store().await;
    let (k, spec) = (key("cap"), BucketSpec::per_minute(5));
    for _ in 0..5 {
        assert_eq!(s.take(&k, spec, 1.0).await.unwrap(), Take::Granted);
    }
    assert!(matches!(
        s.take(&k, spec, 1.0).await.unwrap(),
        Take::Denied { .. }
    ));
}

#[tokio::test]
async fn bucket_denies_and_reports_wait() {
    let s = store().await;
    let (k, spec) = (key("wait"), BucketSpec::per_minute(60));
    assert_eq!(s.take(&k, spec, 60.0).await.unwrap(), Take::Granted);
    match s.take(&k, spec, 1.0).await.unwrap() {
        Take::Denied { wait } => assert!(
            wait >= Duration::from_millis(900) && wait <= Duration::from_millis(1100),
            "{wait:?}"
        ),
        Take::Granted => panic!("must be denied"),
    }
}

#[tokio::test]
async fn refill_uses_redis_time() {
    let s = store().await;
    // 600/min = 10 tokens per second.
    let (k, spec) = (key("refill"), BucketSpec::per_minute(600));
    assert_eq!(s.take(&k, spec, 600.0).await.unwrap(), Take::Granted);
    assert!(matches!(
        s.take(&k, spec, 5.0).await.unwrap(),
        Take::Denied { .. }
    ));
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(s.take(&k, spec, 5.0).await.unwrap(), Take::Granted);
}

#[tokio::test]
async fn partial_acquire_refunds() {
    let s = Arc::new(store().await);
    let lim = TokenBucketLimiter::new(s.clone());
    let model = uuid::Uuid::new_v4().to_string();
    let limits = ModelLimits {
        rpm: 60,
        input_tpm: 1000,
        output_tpm: 1_000_000,
    };
    let req = |est: u32, within: Duration| LimitRequest {
        provider: ProviderId::new("anthropic"),
        model: model.clone(),
        limits,
        est_input: est,
        max_output: 10,
        deadline: Instant::now() + within,
    };
    let cancel = CancellationToken::new();
    lim.acquire(&req(900, Duration::from_secs(5)), &cancel)
        .await
        .unwrap();
    assert!(lim
        .acquire(&req(900, Duration::from_millis(50)), &cancel)
        .await
        .is_err());
    let spec = BucketSpec::per_minute(60);
    let k = format!("rg:rl:anthropic:{model}:req");
    for _ in 0..59 {
        assert_eq!(s.take(&k, spec, 1.0).await.unwrap(), Take::Granted);
    }
    assert!(matches!(
        s.take(&k, spec, 1.0).await.unwrap(),
        Take::Denied { .. }
    ));
}

#[tokio::test]
async fn reconcile_refunds_unused_output() {
    let s = Arc::new(store().await);
    let lim = TokenBucketLimiter::new(s.clone());
    let model = uuid::Uuid::new_v4().to_string();
    let limits = ModelLimits {
        rpm: 60,
        input_tpm: 1_000_000,
        output_tpm: 1000,
    };
    let r = lim
        .acquire(
            &LimitRequest {
                provider: ProviderId::new("anthropic"),
                model: model.clone(),
                limits,
                est_input: 10,
                max_output: 1000,
                deadline: Instant::now() + Duration::from_secs(5),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    lim.reconcile(&r, 10, 100).await;
    let k = format!("rg:rl:anthropic:{model}:out_tok");
    assert_eq!(
        s.take(&k, BucketSpec::per_minute(1000), 850.0)
            .await
            .unwrap(),
        Take::Granted
    );
}

#[tokio::test]
async fn two_gateways_share_one_bucket() {
    // Four independent connections (as four gateway instances would have) hammer one bucket for
    // 3 seconds. A 60/min bucket holds 60 and refills 1/s, so about 63 grants are possible in
    // total; without sharing each instance would get its own 60.
    let k = key("shared");
    let spec = BucketSpec::per_minute(60);
    let granted = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let s = store().await;
        let (k, granted) = (k.clone(), granted.clone());
        handles.push(tokio::spawn(async move {
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if s.take(&k, spec, 1.0).await.unwrap() == Take::Granted {
                    granted.fetch_add(1, Ordering::SeqCst);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let total = granted.load(Ordering::SeqCst);
    assert!((60..=66).contains(&total), "total grants {total}");
}

#[tokio::test]
async fn unreachable_redis_falls_back_to_local() {
    // Port 1 refuses connections: connecting fails, so use a store that is down by construction.
    assert!(RedisBuckets::connect("redis://127.0.0.1:1").await.is_err());
    let live = Arc::new(store().await);
    let fb =
        FallbackStore::new(live, 4).with_timing(Duration::from_millis(50), Duration::from_secs(30));
    let k = key("fb");
    assert_eq!(
        fb.take(&k, BucketSpec::per_minute(8), 1.0).await.unwrap(),
        Take::Granted
    );
    assert_eq!(fb.degraded_total(), 0);
}
