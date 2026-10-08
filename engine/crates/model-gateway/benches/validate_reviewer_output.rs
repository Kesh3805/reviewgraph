#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! Validation of a ~30 KB reviewer-shaped output (target: under 1 ms).

use criterion::{criterion_group, criterion_main, Criterion};
use model_gateway::{OutputSchema, SchemaValidators};
use serde_json::{json, Value};

fn schema() -> OutputSchema {
    let evidence = json!({
        "type": "object", "additionalProperties": false,
        "required": ["ref", "quote", "explanation"],
        "properties": {
            "ref": {"type": "string", "pattern": "^[SNTRCD][0-9]+$"},
            "quote": {"type": "string", "maxLength": 300},
            "explanation": {"type": "string", "maxLength": 400}
        }
    });
    OutputSchema::new(
        "bench_output",
        "1",
        json!({
            "type": "object", "additionalProperties": false, "required": ["findings"],
            "properties": {"findings": {"type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["title", "description", "evidence"],
                "properties": {
                    "title": {"type": "string", "maxLength": 120},
                    "description": {"type": "string", "maxLength": 2000},
                    "evidence": {"type": "array", "items": evidence}
                }
            }}}
        }),
    )
}

fn output() -> Value {
    let finding = |i: usize| {
        json!({
            "title": format!("finding {i}"),
            "description": "d".repeat(1_800),
            "evidence": (0..4).map(|j| json!({
                "ref": format!("S{j}"), "quote": "q".repeat(250), "explanation": "e".repeat(300)
            })).collect::<Vec<_>>()
        })
    };
    json!({ "findings": (0..10).map(finding).collect::<Vec<_>>() })
}

fn bench(c: &mut Criterion) {
    let v = SchemaValidators::new();
    let s = schema();
    let out = output();
    assert!(out.to_string().len() > 30_000);
    c.bench_function("validate_reviewer_output", |b| {
        b.iter(|| v.validate(&s, &out).expect("valid schema"))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
