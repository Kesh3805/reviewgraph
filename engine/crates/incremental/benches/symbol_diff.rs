#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Baseline for the per-file symbol diff (SID-004): two 2,000-symbol units with 5% of the symbols
//! changed. Recorded, not gated, until PERF-002.

use analysis_ir::hashing::{hash_tokens, shingles, HashKind, Token};
use analysis_ir::{IrSymbol, LocalId, ParseStatus, ParsedUnit};
use criterion::{criterion_group, criterion_main, Criterion};
use incremental::symbol_diff::diff_units;
use review_core::language::Language;
use review_core::location::{ContentHash, Position, RepoPath, SourceRange};
use review_core::symbol::{Hash128, ModulePath, SymbolKind};
use review_core::symbol_id::module_path_for;

const SYMBOLS: usize = 2_000;
const CHANGED: usize = SYMBOLS / 20;

fn range(index: usize) -> SourceRange {
    let line = u32::try_from(index + 1).unwrap_or(1);
    SourceRange {
        start: Position { line, column: 0 },
        end: Position {
            line: line + 1,
            column: 0,
        },
    }
}

/// A unit whose every `changed_every`-th symbol has a different body hash, so the diff sees exactly
/// `CHANGED` modified symbols.
fn unit(changed_every: usize) -> ParsedUnit {
    let path = RepoPath::new("src/big/service.ts").unwrap();
    let module_path: ModulePath = module_path_for(&path, Language::Typescript);
    let mut symbols: Vec<IrSymbol> = Vec::with_capacity(SYMBOLS);
    for index in 0..SYMBOLS {
        let name = format!("method{index}");
        let tokens: Vec<Token> = (0..12).map(|i| Token::ident(format!("t{i}"))).collect();
        let edited = changed_every > 0 && index % changed_every == 0;
        let body_domain = if edited { "edit" } else { "body" };
        symbols.push(IrSymbol {
            body_hash: Hash128::of(body_domain, &[index as u8]),
            signature_hash: hash_tokens(HashKind::Signature, &[Token::ident("sig")]),
            attr_hash: hash_tokens(HashKind::Attributes, &[]),
            body_shingles: shingles(&tokens, 3),
            body_token_count: 12,
            ..IrSymbol::new(
                LocalId(u32::try_from(index).unwrap_or(0)),
                SymbolKind::Method,
                name.clone(),
                vec!["Service".to_owned(), name],
                range(index),
            )
        });
    }
    ParsedUnit {
        ir_schema: analysis_ir::IR_SCHEMA_VERSION,
        file: path,
        module_path,
        language: Language::Typescript,
        dialect: None,
        content_hash: ContentHash::of(b"big"),
        analyzer: analysis_ir::AnalyzerId {
            name: "bench".to_owned(),
            version: review_core::version::AnalyzerVersion::new(0, 1, 0),
        },
        status: ParseStatus::Ok,
        symbols,
        references: vec![],
        imports: vec![],
        exports: vec![],
        framework: vec![],
        facts: vec![],
        diagnostics: vec![],
        stats: analysis_ir::UnitStats::default(),
    }
}

fn bench_diff(c: &mut Criterion) {
    let base = unit(0);
    let head = unit(SYMBOLS / CHANGED);
    let diff = diff_units(Some(&base), Some(&head));
    assert_eq!(
        diff.counts.modified_body,
        u32::try_from(CHANGED).unwrap_or(0)
    );
    c.bench_function("diff_units/2000_symbols_5%_changed", |b| {
        b.iter(|| {
            diff_units(
                Some(std::hint::black_box(&base)),
                Some(std::hint::black_box(&head)),
            )
        })
    });
}

criterion_group!(benches, bench_diff);
criterion_main!(benches);
