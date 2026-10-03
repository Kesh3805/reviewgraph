#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! Warm fixture lookup (target: p99 under 100 microseconds).

use criterion::{criterion_group, criterion_main, Criterion};
use model_gateway::fixture::{Fixture, FixtureResponse, FixtureStore, FIXTURE_VERSION};
use model_gateway::{FinishReason, ModelOutput, TaskType, Usage};
use serde_json::json;

fn bench(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt");
    let dir = tempfile::tempdir().expect("tmp");
    let store = FixtureStore::new(dir.path());
    let hash = "ab".repeat(32);
    let fixture = Fixture {
        fixture_version: FIXTURE_VERSION,
        request_hash: hash.clone(),
        provider: "anthropic".into(),
        model: "m".into(),
        task: "correctness_review".into(),
        prompt_id: "p".into(),
        prompt_version: "v1".into(),
        schema_hash: None,
        synthetic: false,
        recorded_at: "2026-01-01T00:00:00Z".into(),
        engine_git_sha: None,
        response: FixtureResponse {
            output: ModelOutput::Json(json!({"ok": true})),
            usage: Usage::default(),
            finish_reason: FinishReason::Complete,
        },
        latency_ms: 0,
    };
    rt.block_on(store.write(TaskType::CorrectnessReview, &fixture, false))
        .expect("write");
    rt.block_on(store.lookup(TaskType::CorrectnessReview, &hash, "anthropic", "m"))
        .expect("warm");
    c.bench_function("replay_lookup", |b| {
        b.iter(|| {
            rt.block_on(store.lookup(TaskType::CorrectnessReview, &hash, "anthropic", "m"))
                .expect("lookup")
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
