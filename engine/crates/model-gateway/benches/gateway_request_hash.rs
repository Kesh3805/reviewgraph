#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! `request_hash` on a ~40k-token input (target: under 2 ms).

use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use model_gateway::{
    request_hash, CallBudget, InputSection, ModelRequest, ModelTier, StructuredInput, SystemPrompt,
    TaskType, TenantScope,
};
use review_core::ids::{OrganizationId, RepositoryId};
use serde_json::json;

fn bench(c: &mut Criterion) {
    // ~140 KB of JSON is roughly 40k tokens.
    let chunk = "fn example(a: u32) -> u32 { a + 1 }\n".repeat(100);
    let sections: Vec<InputSection> = (0..40)
        .map(|i| {
            InputSection::new(
                format!("file_{i}"),
                json!({ "path": format!("src/{i}.ts"), "text": chunk }),
            )
        })
        .collect();
    let input = StructuredInput::new(
        SystemPrompt {
            prompt_id: "p".into(),
            prompt_version: "v1".into(),
            prompt_sha: "s".into(),
            text: Arc::from("system"),
        },
        sections,
    );
    let req = ModelRequest::new(
        TaskType::CorrectnessReview,
        ModelTier::ReviewReasoner,
        input,
        TenantScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        },
        CallBudget::within(Duration::from_secs(60)),
    );
    c.bench_function("gateway_request_hash", |b| b.iter(|| request_hash(&req)));
}

criterion_group!(benches, bench);
criterion_main!(benches);
