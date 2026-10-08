#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Baseline for the rename matcher (SID-005): 5,000 removed against 5,000 added symbols with 10%
//! true renames, plus the degraded 50k x 50k path. Recorded, not gated, until PERF-002.

use criterion::{criterion_group, criterion_main, Criterion};
use incremental::matcher::{match_symbols, MatcherConfig, RenameHints};
use review_core::ids::SymbolKey;
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::matcher::SymbolRef;
use review_core::symbol::{Hash128, ShingleSet, SymbolKind};
use review_core::symbol_id::{module_path_for, SymbolIdParts};

fn symbol(index: usize, renamed: bool, body: u8, module: &str) -> SymbolRef {
    let path = RepoPath::new(format!("{module}.ts")).unwrap();
    let module_path = module_path_for(&path, Language::Typescript);
    let name = if renamed {
        format!("renamed{index}")
    } else {
        format!("fn{index}")
    };
    let id = SymbolIdParts::new(
        "ts",
        module_path.clone(),
        vec![name.clone()],
        SymbolKind::Function,
    )
    .format()
    .expect("valid parts");
    let tokens: Vec<u32> = (0..32u32).map(|i| u32::from(body) * 100 + i).collect();
    SymbolRef {
        key: SymbolKey::of(&id),
        id,
        kind: SymbolKind::Function,
        name,
        qualified_name: vec![format!("fn{index}")],
        module_path,
        parent_id: None,
        signature_hash: Hash128::of("sig", &[index as u8]),
        body_hash: Hash128::of("body", &[body]),
        body_token_count: 32,
        shingles: ShingleSet::of(tokens),
    }
}

fn pools(size: usize, renames: usize) -> (Vec<SymbolRef>, Vec<SymbolRef>) {
    let mut removed = Vec::with_capacity(size);
    let mut added = Vec::with_capacity(size);
    for index in 0..size {
        let is_rename = index < renames;
        let body = (index % 251) as u8;
        removed.push(symbol(
            index,
            is_rename,
            body,
            &format!("src/base/f{index}"),
        ));
        added.push(symbol(
            index,
            is_rename,
            body,
            &format!("src/head/f{index}"),
        ));
    }
    (removed, added)
}

fn bench_matcher(c: &mut Criterion) {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    let (removed, added) = pools(5_000, 500);
    let result = match_symbols(&removed, &added, &cfg, &hints);
    assert!(
        result.matches.len() >= 500,
        "the 500 renames must be found: {}",
        result.matches.len()
    );
    c.bench_function("match_symbols/5000x5000_10%_renames", |b| {
        b.iter(|| match_symbols(&removed, &added, &cfg, &hints))
    });

    let (big_removed, big_added) = pools(50_000, 5_000);
    let degraded = match_symbols(&big_removed, &big_added, &cfg, &hints);
    assert!(degraded.degraded, "50k pools must run in degraded mode");
    c.bench_function("match_symbols/50000x50000_degraded", |b| {
        b.iter(|| match_symbols(&big_removed, &big_added, &cfg, &hints))
    });
}

criterion_group!(benches, bench_matcher);
criterion_main!(benches);
