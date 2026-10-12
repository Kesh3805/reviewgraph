#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-006: embedding unit builders.

mod common;

use common::{authorize_symbol, function_symbol, path, snapshot};
use review_core::ids::RepositoryId;
use review_core::symbol::SymbolKind;
use semantic::units::{
    code_chunks, content_hash, convention_unit, doc_units, symbol_summary, ConventionInput,
    DocInput, UnitKind, CHUNK_MAX_CHARS, DOC_MAX_CHARS, SUMMARY_MAX_CHARS, UNIT_TEMPLATE_VERSION,
};

#[test]
fn auth_service_authorize_summary_snapshot() {
    let sym = authorize_symbol();
    let unit = symbol_summary(&sym, RepositoryId::new(), snapshot()).unwrap();
    assert_eq!(
        unit.text,
        "method AuthService.authorize\n\
         signature: async authorize(user: User, resource: Resource): Promise<boolean>\n\
         module: src/auth/auth.service\n\
         decorators: -\n\
         calls: PermissionService.check\n\
         called by: AdminService.updateUser\n\
         doc: Checks whether the user may access the resource."
    );
    assert_eq!(unit.kind, UnitKind::SymbolSummary);
    assert_eq!(unit.key, sym.symbol_key().to_string());
    assert_eq!(unit.symbol_key, Some(sym.symbol_key()));
    assert_eq!(unit.module.as_deref(), Some("src/auth/auth.service"));
    assert_eq!((unit.start_line, unit.end_line), (Some(10), Some(13)));
}

#[test]
fn summary_lists_are_ranked_and_capped() {
    let mut sym = authorize_symbol();
    sym.callees = (0..15)
        .map(|i| (format!("callee{i:02}"), i as f32 / 100.0))
        .collect();
    sym.callers = (0..8).rev().map(|i| format!("caller{i}")).collect();
    sym.doc = Some("x".repeat(5_000));
    let unit = symbol_summary(&sym, RepositoryId::new(), snapshot()).unwrap();
    assert!(unit.text.contains("calls: callee14, callee13,"));
    assert!(!unit.text.contains("callee04"));
    assert!(unit
        .text
        .contains("called by: caller0, caller1, caller2, caller3, caller4\n"));
    assert!(unit.text.chars().count() <= SUMMARY_MAX_CHARS);
}

#[test]
fn trivial_getter_skipped() {
    let mut getter = authorize_symbol();
    getter.kind = SymbolKind::Getter;
    getter.is_private = true;
    getter.start_line = 5;
    getter.end_line = 6;
    assert!(symbol_summary(&getter, RepositoryId::new(), snapshot()).is_none());
    getter.is_private = false;
    assert!(symbol_summary(&getter, RepositoryId::new(), snapshot()).is_some());
    let mut field = authorize_symbol();
    field.kind = SymbolKind::Property;
    assert!(symbol_summary(&field, RepositoryId::new(), snapshot()).is_none());
}

#[test]
fn generated_file_skipped() {
    let mut sym = function_symbol("generatedClient", "src/gen/client.ts", 20);
    sym.is_generated = true;
    assert!(symbol_summary(&sym, RepositoryId::new(), snapshot()).is_none());
    assert!(code_chunks(&sym, RepositoryId::new(), snapshot()).is_empty());
}

#[test]
fn chunking_60_lines_overlap_10() {
    // 118 body lines + 2 brace lines = 120 lines.
    let sym = function_symbol("bigFunction", "src/big.ts", 118);
    let repo = RepositoryId::new();
    let chunks = code_chunks(&sym, repo, snapshot());
    let ranges: Vec<(u32, u32)> = chunks
        .iter()
        .map(|c| (c.start_line.unwrap(), c.end_line.unwrap()))
        .collect();
    assert_eq!(ranges, vec![(1, 60), (51, 110), (101, 120)]);
    for c in &chunks {
        assert_eq!(c.kind, UnitKind::CodeChunk);
        assert!(c
            .text
            .starts_with("function bigFunction(input: string): void\n"));
        assert!(c.text.chars().count() <= CHUNK_MAX_CHARS);
        assert_eq!(c.symbol_key, Some(sym.symbol_key()));
    }
    let keys: std::collections::BTreeSet<&str> = chunks.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys.len(), 3);
    // Degraded parse: summary only.
    let mut degraded = sym.clone();
    degraded.body = None;
    assert!(code_chunks(&degraded, repo, snapshot()).is_empty());
    assert!(symbol_summary(&degraded, repo, snapshot()).is_some());
    // Short bodies are not chunked.
    assert!(code_chunks(&function_symbol("tiny", "src/t.ts", 1), repo, snapshot()).is_empty());
}

#[test]
fn doc_split_on_headings() {
    let doc = DocInput {
        path: path("docs/auth.md"),
        text: "Intro line.\n\n# Auth\nAll admin calls authorize first.\n\n## Tokens\nTokens expire.\n\
               ```\n# not a heading\n```\n### Deep\nDeep text.\n#### Deeper\nstill deep.\n## Empty\n\n"
            .into(),
    };
    let units = doc_units(&doc, RepositoryId::new(), snapshot());
    let heads: Vec<&str> = units
        .iter()
        .map(|u| u.text.lines().next().unwrap())
        .collect();
    assert_eq!(
        heads,
        vec![
            "Intro line.",
            "Auth",
            "Auth > Tokens",
            "Auth > Tokens > Deep"
        ]
    );
    assert!(units[2].text.contains("# not a heading"));
    assert!(units[3].text.contains("#### Deeper"));
    assert_eq!(units[1].start_line, Some(4));

    let long = DocInput {
        path: path("README.md"),
        text: {
            let line = format!("{}\n", "word ".repeat(40).trim_end());
            format!("# Big\n{}", line.repeat(30))
        },
    };
    let parts = doc_units(&long, RepositoryId::new(), snapshot());
    assert!(parts.len() > 1);
    assert!(parts
        .iter()
        .all(|u| u.text.chars().count() <= DOC_MAX_CHARS));
    let keys: std::collections::BTreeSet<&str> = parts.iter().map(|u| u.key.as_str()).collect();
    assert_eq!(keys.len(), parts.len());
}

#[test]
fn convention_unit_text() {
    let c = ConventionInput {
        id: "di-constructor".into(),
        rule: "Services receive dependencies through constructor injection".into(),
        scope: "src/**/*.service.ts".into(),
        examples: vec![
            "AuthService".into(),
            "AdminService".into(),
            "ReportService".into(),
        ],
        confidence: 0.875,
    };
    let unit = convention_unit(&c, RepositoryId::new(), snapshot());
    assert_eq!(
        unit.text,
        "Services receive dependencies through constructor injection\n\
         scope: src/**/*.service.ts\n\
         examples: AuthService, AdminService\n\
         confidence: 0.88"
    );
    assert_eq!(unit.key, "di-constructor");
    assert_eq!(unit.kind, UnitKind::Convention);
}

#[test]
fn content_hash_excludes_snapshot() {
    let sym = authorize_symbol();
    let repo = RepositoryId::new();
    let a = symbol_summary(&sym, repo, snapshot()).unwrap();
    let b = symbol_summary(&sym, repo, snapshot()).unwrap();
    assert_ne!(a.snapshot_id, b.snapshot_id);
    assert_eq!(a.content_hash, b.content_hash);
    let mut changed = sym.clone();
    changed.callers.push("UserController.update".into());
    let c = symbol_summary(&changed, repo, snapshot()).unwrap();
    assert_ne!(a.content_hash, c.content_hash);
}

#[test]
fn secret_literal_redacted_in_text() {
    let mut sym = function_symbol("connect", "src/db.ts", 6);
    sym.body = Some(
        "function connect() {\n  const password = \"hunter2hunter2\";\n  const key = \"AKIAABCDEFGHIJKLMNOP\";\n  return open(password, key);\n  // done\n}"
            .into(),
    );
    let chunks = code_chunks(&sym, RepositoryId::new(), snapshot());
    assert_eq!(chunks.len(), 1);
    assert!(
        !chunks[0].text.contains("hunter2hunter2"),
        "{}",
        chunks[0].text
    );
    assert!(!chunks[0].text.contains("AKIAABCDEFGHIJKLMNOP"));
    assert!(chunks[0].text.contains("«redacted:"));
    // Files with a secrets signal produce no chunks at all.
    sym.has_secrets = true;
    assert!(code_chunks(&sym, RepositoryId::new(), snapshot()).is_empty());
}

#[test]
fn template_version_changes_hash() {
    let text = "method AuthService.authorize";
    assert_ne!(
        content_hash(UnitKind::SymbolSummary, UNIT_TEMPLATE_VERSION, text),
        content_hash(UnitKind::SymbolSummary, UNIT_TEMPLATE_VERSION + 1, text)
    );
    assert_ne!(
        content_hash(UnitKind::SymbolSummary, UNIT_TEMPLATE_VERSION, text),
        content_hash(UnitKind::CodeChunk, UNIT_TEMPLATE_VERSION, text)
    );
}
