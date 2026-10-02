//! Baseline for `SymbolKey::of` on a 64-byte symbol ID. Target: < 300 ns/op (DOM-001).

use criterion::{criterion_group, criterion_main, Criterion};
use review_core::ids::{SymbolId, SymbolKey};
use std::hint::black_box;

fn symbol_key(c: &mut Criterion) {
    let id = SymbolId::from_canonical_unchecked(
        "ts:src/auth/auth.service#AuthService.authorize/method.overload0",
    );
    c.bench_function("SymbolKey::of/64B", |b| {
        b.iter(|| SymbolKey::of(black_box(&id)))
    });
}

criterion_group!(benches, symbol_key);
criterion_main!(benches);
