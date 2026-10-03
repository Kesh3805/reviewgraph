#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! `route()` cost (target: under 5 microseconds).

use std::collections::HashSet;

use criterion::{criterion_group, criterion_main, Criterion};
use model_gateway::{
    route, ModelTier, PrivacyClass, ProviderId, RiskBand, RouteQuery, RoutingTable,
};

fn bench(c: &mut Criterion) {
    let table = RoutingTable::default_table(&|_| None).expect("table");
    let registered: HashSet<ProviderId> = [ProviderId::new("anthropic")].into_iter().collect();
    let q = RouteQuery {
        tier: ModelTier::ReviewReasoner,
        risk_band: RiskBand::High,
        privacy: PrivacyClass::Standard,
        est_input_tokens: 20_000,
        max_output_tokens: 4_000,
        remaining_budget_fraction: 1.0,
        schema_strict_ok: registered.clone(),
    };
    c.bench_function("router_route", |b| {
        b.iter(|| route(&table, &registered, &q))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
