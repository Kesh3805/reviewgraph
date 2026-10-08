#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use model_gateway::accounting::ledger::ChannelLedger;
use model_gateway::cache::{cache_key, CacheEntry};
use model_gateway::{
    CachePolicy, CallBudget, FinishReason, GatewayBuilder, GatewayError, InputSection, LedgerSink,
    MemoryCache, MemoryLedger, ModelGateway, ModelOutput, ModelRequest, ModelTier, PermanentKind,
    PriceTable, ProviderAdapter, ProviderId, ProviderRequest, ProviderResponse, ResponseCache,
    RetryPolicy, RouteCandidate, ServedFrom, StaticRouter, TaskType, TransientKind, Usage,
    WorstCasePricer,
};
use review_core::ids::OrganizationId;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use common::{input, tenant};

fn usage(u: u32, w: u32, r: u32, o: u32) -> Usage {
    Usage {
        input_uncached: u,
        cache_write: w,
        cache_read: r,
        output: o,
        reasoning: 0,
    }
}

#[test]
fn cost_formula_matches_hand_calculation() {
    let t = PriceTable::default_table().expect("prices");
    // haiku: 1000*1 + 2000*1.25 + 40000*0.10 + 500*5 = 10000 micro-USD
    assert_eq!(
        t.cost_micros(
            "anthropic",
            "claude-haiku-4-5",
            &usage(1000, 2000, 40_000, 500)
        ),
        Some(10_000)
    );
    // sonnet: 1000*2 + 0 + 10000*0.20 + 100*10 = 5000
    assert_eq!(
        t.cost_micros(
            "anthropic",
            "claude-sonnet-5-5",
            &usage(1000, 0, 10_000, 100)
        ),
        Some(5_000)
    );
    // opus 5.5: 1000*4 + 1000*5 + 50000*0.20 + 200*20 = 23000
    assert_eq!(
        t.cost_micros(
            "anthropic",
            "claude-opus-5-5",
            &usage(1000, 1000, 50_000, 200)
        ),
        Some(23_000)
    );
    assert_eq!(
        t.cost_micros("anthropic", "claude-haiku-4-5", &Usage::default()),
        Some(0)
    );
}

fn custom(yaml_prices: &str) -> PriceTable {
    PriceTable::from_yaml(&format!("version: 1\nprices:\n{yaml_prices}")).expect("table")
}

#[test]
fn cost_rounds_half_up() {
    let t = custom(
        "  - { provider: p, model: m, usd_per_mtok: { input: \"0.5\", output: \"0.5\", cache_write: \"0.5\", cache_read: \"0.5\" }, as_of: \"2026-10-03\", source: x }\n",
    );
    assert_eq!(t.cost_micros("p", "m", &usage(1, 0, 0, 0)), Some(1)); // 0.5 -> 1
    assert_eq!(t.cost_micros("p", "m", &usage(3, 0, 0, 0)), Some(2)); // 1.5 -> 2
    assert_eq!(t.cost_micros("p", "m", &usage(0, 0, 0, 5)), Some(3)); // 2.5 -> 3
}

#[test]
fn openai_cached_tokens_not_double_counted() {
    let t = custom(
        "  - { provider: openai, model: m, usd_per_mtok: { input: \"2\", output: \"8\", cache_write: \"2\", cache_read: \"0.5\" }, as_of: \"2026-10-03\", source: x }\n",
    );
    // OpenAI reported input_tokens=5000 of which 4096 cached: the adapter normalises to
    // uncached=904, cache_read=4096, so the cached part is billed once, at the cache rate.
    let cost = t
        .cost_micros("openai", "m", &usage(904, 0, 4096, 70))
        .expect("priced");
    assert_eq!(cost, 904 * 2 + 2048 + 560);
}

#[test]
fn unpriced_model_yields_none_and_worst_case_uses_dearest_sibling() {
    let t = PriceTable::default_table().expect("prices");
    assert_eq!(
        t.cost_micros("openai", "whatever", &usage(1, 1, 1, 1)),
        None
    );
    assert_eq!(
        t.cost_micros("anthropic", "claude-mystery", &usage(1, 1, 1, 1)),
        None
    );
    // Worst case for an unknown anthropic model is bounded by opus (output $20, input/cache-write $5).
    let w = t
        .worst_case_micros(&ProviderId::new("anthropic"), "claude-mystery", 1000, 100)
        .expect("bound");
    assert_eq!(w, 1000 * 5 + 100 * 20);
    assert_eq!(
        t.worst_case_micros(&ProviderId::new("openai"), "x", 1, 1),
        None
    );
}

#[test]
fn prices_as_of_staleness_check() {
    let t = PriceTable::default_table().expect("prices");
    let today = chrono::Utc::now().date_naive();
    let stale = t.stale(today, model_gateway::accounting::MAX_PRICE_AGE_DAYS);
    assert!(
        stale.is_empty(),
        "prices older than 120 days, re-check the pricing pages and update config/prices.yaml: {:?}",
        stale.iter().map(|e| &e.model).collect::<Vec<_>>()
    );
    // The check itself works.
    let future = NaiveDate::from_ymd_opt(2030, 1, 1).expect("date");
    assert_eq!(t.stale(future, 120).len(), t.entries().len());
}

#[test]
fn every_default_model_has_source_and_date() {
    let t = PriceTable::default_table().expect("prices");
    for m in ["claude-haiku-4-5", "claude-sonnet-5-5", "claude-opus-5-5"] {
        let e = t.get("anthropic", m).expect(m);
        assert!(e.source.starts_with("https://"));
    }
}

// ---- gateway wiring ----

struct Script {
    results: Mutex<VecDeque<Result<ProviderResponse, GatewayError>>>,
    calls: Arc<AtomicUsize>,
}

fn ok_response(model: &str, u: Usage) -> ProviderResponse {
    ProviderResponse {
        output: ModelOutput::Json(json!({"ok": true})),
        usage: u,
        finish_reason: FinishReason::Complete,
        model: model.into(),
        provider_request_id: None,
        tool_use_id: None,
        served_from: ServedFrom::Live,
    }
}

#[async_trait]
impl ProviderAdapter for Script {
    fn provider(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    async fn send(&self, _r: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut q = self.results.lock().unwrap();
        if q.len() > 1 {
            q.pop_front().unwrap()
        } else {
            match q.front().unwrap() {
                Ok(r) => Ok(r.clone()),
                Err(_) => Err(GatewayError::Permanent {
                    kind: PermanentKind::Unknown,
                    provider: None,
                    detail: "scripted".into(),
                }),
            }
        }
    }
}

struct Rig {
    gw: model_gateway::Gateway,
    calls: Arc<AtomicUsize>,
    ledger: Arc<MemoryLedger>,
    cache: Arc<MemoryCache>,
}

fn rig(results: Vec<Result<ProviderResponse, GatewayError>>) -> Rig {
    let calls = Arc::new(AtomicUsize::new(0));
    let ledger = Arc::new(MemoryLedger::new());
    let cache = Arc::new(MemoryCache::new());
    let router = StaticRouter::new()
        .with_route(
            ModelTier::Classifier,
            vec![RouteCandidate {
                provider: ProviderId::new("anthropic"),
                model: "claude-haiku-4-5".into(),
                max_context: 200_000,
                supports_reasoning: false,
            }],
        )
        .with_route(
            ModelTier::ReviewReasoner,
            vec![RouteCandidate {
                provider: ProviderId::new("anthropic"),
                model: "claude-sonnet-5-5".into(),
                max_context: 200_000,
                supports_reasoning: false,
            }],
        );
    let gw = GatewayBuilder::new()
        .redactor(Arc::new(model_gateway::DefaultRedactor::new()))
        .adapter(Arc::new(Script {
            results: Mutex::new(results.into()),
            calls: calls.clone(),
        }))
        .router(Arc::new(router))
        .prices(Arc::new(PriceTable::default_table().expect("prices")))
        .ledger(ledger.clone())
        .response_cache(cache.clone())
        .retry_policy(
            RetryPolicy::default().with_delays(Duration::from_millis(1), Duration::from_millis(2)),
        )
        .build()
        .expect("build");
    Rig {
        gw,
        calls,
        ledger,
        cache,
    }
}

fn classify_req(org: Option<OrganizationId>, ttl: Duration) -> ModelRequest {
    let mut t = tenant();
    if let Some(o) = org {
        t.organization_id = o;
    }
    ModelRequest::new(
        TaskType::IntentClassification,
        ModelTier::Classifier,
        input(),
        t,
        CallBudget::within(Duration::from_secs(60)),
    )
    .with_cache(CachePolicy::PromptAndResponse { ttl })
}

#[tokio::test]
async fn cache_bypassed_for_review_tasks() {
    let r = rig(vec![Ok(ok_response(
        "claude-sonnet-5-5",
        usage(100, 0, 0, 10),
    ))]);
    let req = || {
        ModelRequest::new(
            TaskType::CorrectnessReview,
            ModelTier::ReviewReasoner,
            input(),
            tenant(),
            CallBudget::within(Duration::from_secs(60)),
        )
        .with_cache(CachePolicy::PromptAndResponse {
            ttl: Duration::from_secs(3600),
        })
    };
    r.gw.call(req(), CancellationToken::new())
        .await
        .expect("ok");
    r.gw.call(req(), CancellationToken::new())
        .await
        .expect("ok");
    assert_eq!(r.calls.load(Ordering::SeqCst), 2);
    assert!(r.cache.is_empty());
}

#[tokio::test]
async fn cache_hit_returns_zero_cost() {
    let r = rig(vec![Ok(ok_response(
        "claude-haiku-4-5",
        usage(1000, 0, 0, 500),
    ))]);
    let org = OrganizationId::new();
    let live =
        r.gw.call(
            classify_req(Some(org), Duration::from_secs(3600)),
            CancellationToken::new(),
        )
        .await
        .expect("live");
    assert_eq!(live.cost_usd_micros, Some(1000 + 2500));
    assert_eq!(live.served_from, ServedFrom::Live);

    let hit =
        r.gw.call(
            classify_req(Some(org), Duration::from_secs(3600)),
            CancellationToken::new(),
        )
        .await
        .expect("hit");
    assert_eq!(hit.served_from, ServedFrom::ResponseCache);
    assert_eq!(hit.cost_usd_micros, Some(0));
    assert_eq!(hit.usage, Usage::default());
    assert_eq!(hit.usage_original, Some(usage(1000, 0, 0, 500)));
    assert_eq!(hit.output, live.output);
    assert_eq!(r.calls.load(Ordering::SeqCst), 1);
    let rows = r.ledger.records();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].served_from, ServedFrom::ResponseCache);
    assert_eq!(rows[1].cost_usd_micros, Some(0));
}

#[tokio::test]
async fn cache_never_crosses_tenants() {
    let r = rig(vec![Ok(ok_response(
        "claude-haiku-4-5",
        usage(10, 0, 0, 5),
    ))]);
    r.gw.call(
        classify_req(None, Duration::from_secs(3600)),
        CancellationToken::new(),
    )
    .await
    .expect("org A");
    let b =
        r.gw.call(
            classify_req(None, Duration::from_secs(3600)),
            CancellationToken::new(),
        )
        .await
        .expect("org B");
    assert_eq!(
        b.served_from,
        ServedFrom::Live,
        "a different organisation must miss"
    );
    assert_eq!(r.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cache_expired_is_miss() {
    let r = rig(vec![Ok(ok_response(
        "claude-haiku-4-5",
        usage(10, 0, 0, 5),
    ))]);
    let org = OrganizationId::new();
    r.gw.call(
        classify_req(Some(org), Duration::ZERO),
        CancellationToken::new(),
    )
    .await
    .expect("live");
    r.gw.call(
        classify_req(Some(org), Duration::ZERO),
        CancellationToken::new(),
    )
    .await
    .expect("live again");
    assert_eq!(r.calls.load(Ordering::SeqCst), 2);
    assert_eq!(r.cache.purge_expired().await.expect("purge"), 1);
}

#[tokio::test]
async fn cache_put_conflict_is_noop() {
    let c = MemoryCache::new();
    let org = OrganizationId::new();
    let key = cache_key(org, "h", "anthropic", "m", None);
    let entry = |n: i64| CacheEntry {
        request_hash: "h".into(),
        provider: "anthropic".into(),
        model: "m".into(),
        prompt_version: "v1".into(),
        schema_hash: None,
        output: ModelOutput::Json(json!({"n": n})),
        usage: Usage::default(),
    };
    c.put(org, &key, entry(1), Duration::from_secs(60))
        .await
        .expect("put");
    c.put(org, &key, entry(2), Duration::from_secs(60))
        .await
        .expect("put");
    assert_eq!(
        c.get(org, &key).await.expect("get").expect("hit").output,
        ModelOutput::Json(json!({"n": 1}))
    );
    assert!(c
        .get(OrganizationId::new(), &key)
        .await
        .expect("get")
        .is_none());
}

#[test]
fn cache_key_depends_on_every_part() {
    let org = OrganizationId::new();
    let base = cache_key(org, "h", "anthropic", "m", Some("s"));
    assert_ne!(
        base,
        cache_key(OrganizationId::new(), "h", "anthropic", "m", Some("s"))
    );
    assert_ne!(base, cache_key(org, "h2", "anthropic", "m", Some("s")));
    assert_ne!(base, cache_key(org, "h", "openai", "m", Some("s")));
    assert_ne!(base, cache_key(org, "h", "anthropic", "m2", Some("s")));
    assert_ne!(base, cache_key(org, "h", "anthropic", "m", None));
    // Concatenation ambiguity is excluded by length prefixes.
    assert_ne!(
        cache_key(org, "ab", "c", "m", None),
        cache_key(org, "a", "bc", "m", None)
    );
}

#[tokio::test]
async fn ledger_has_one_row_per_attempt_and_sums_to_response() {
    let r = rig(vec![
        Err(GatewayError::Transient {
            kind: TransientKind::Timeout,
            provider: None,
        }),
        Ok(ok_response("claude-haiku-4-5", usage(2000, 0, 0, 100))),
    ]);
    let resp =
        r.gw.call(
            classify_req(None, Duration::from_secs(60)),
            CancellationToken::new(),
        )
        .await
        .expect("ok");
    assert_eq!(resp.attempts, 2);
    let rows = r.ledger.records();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].outcome, "transient");
    assert_eq!(rows[0].attempt, 1);
    assert_eq!(rows[1].outcome, "ok");
    assert_eq!(rows[1].attempt, 2);
    assert_eq!(rows[1].prices_as_of, NaiveDate::from_ymd_opt(2026, 10, 3));
    assert_eq!(
        r.ledger.total_cost_micros(),
        resp.cost_usd_micros.expect("cost")
    );
}

#[test]
fn ledger_full_channel_does_not_block() {
    let (ledger, _rx) = ChannelLedger::channel(1);
    let rec = || model_gateway::LedgerRecord {
        id: uuid::Uuid::now_v7(),
        organization_id: OrganizationId::new(),
        repository_id: review_core::ids::RepositoryId::new(),
        review_run_id: None,
        reviewer_run_id: None,
        task: "t".into(),
        tier: "t".into(),
        provider: "p".into(),
        model: "m".into(),
        attempt: 1,
        request_hash: "h".into(),
        served_from: ServedFrom::Live,
        outcome: "ok".into(),
        usage: Usage::default(),
        cost_usd_micros: None,
        latency_ms: 0,
        prices_as_of: None,
    };
    for _ in 0..5 {
        ledger.record(rec());
    }
    assert_eq!(ledger.dropped_total(), 4);
}

#[test]
fn input_section_is_unaffected() {
    // The cost path never inspects section content; this guards the Debug redaction contract.
    let s = InputSection::new("x", json!({"secret": "CANARY"}));
    assert!(!format!("{s:?}").contains("CANARY"));
}
