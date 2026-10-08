#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! Pre-send redaction over a ~40k-token input (target: under 3 ms p95).

use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use model_gateway::{
    DefaultRedactor, InputSection, PreSendRedactor, StructuredInput, SystemPrompt,
};
use serde_json::json;

fn input() -> StructuredInput {
    // ~140 KB of code-like text, about 40k tokens.
    let line = "  const total = items.reduce((sum, item) => sum + item.price * item.qty, 0);\n";
    let excerpt = line.repeat(40);
    let sections = (0..48)
        .map(|i| {
            InputSection::new(
                format!("s{i}"),
                json!({ "ref": format!("S{i}"), "body_head": excerpt, "body_base": "" }),
            )
        })
        .collect();
    StructuredInput::new(
        SystemPrompt {
            prompt_id: "bench".into(),
            prompt_version: "v1".into(),
            prompt_sha: "sha".into(),
            text: Arc::from("You review code."),
        },
        sections,
    )
}

fn bench(c: &mut Criterion) {
    let r = DefaultRedactor::new();
    let base = input();
    assert!(base.byte_len() > 130_000);
    c.bench_function("redact_40k", |b| {
        b.iter_batched(
            || base.clone(),
            |mut i| r.redact(&mut i),
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
