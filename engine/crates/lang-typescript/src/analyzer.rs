//! `TypeScriptAnalyzer`: tree-sitter based analysis of TS/TSX/JS files (ADR-006, ADR-007).

use std::time::{Duration, Instant};

use analysis_ir::diagnostic::{DiagCode, DiagSeverity};
use analysis_ir::{
    AnalyzeError, AnalyzerConfig, AnalyzerId, FailReason, LanguageAnalyzer, ParseStatus,
    ParsedUnit, SourceInput, UnitStats, IR_SCHEMA_VERSION,
};
use review_core::language::{Dialect, Language};
use review_core::location::RepoPath;
use review_core::symbol::SymbolKind;
use tree_sitter::ParseOptions;

use crate::diagnostics::{scan_errors, DiagnosticSink};
use crate::parser_pool::{with_parser, Grammar};
use crate::text::Source;
use crate::visit::{self, VisitCtx};
use crate::{ANALYZER_NAME, ANALYZER_VERSION};

/// Bytes inspected for a NUL, which marks binary content.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Dialect from the file name, or `None` for unsupported extensions.
pub fn dialect_of(path: &RepoPath) -> Option<Dialect> {
    let name = path.as_str().rsplit('/').next().unwrap_or(path.as_str());
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".d.ts") || lower.ends_with(".d.mts") || lower.ends_with(".d.cts") {
        return Some(Dialect::Dts);
    }
    match lower.rsplit_once('.').map(|(_, ext)| ext) {
        Some("ts" | "mts" | "cts") => Some(Dialect::Ts),
        Some("tsx") => Some(Dialect::Tsx),
        Some("js") => Some(Dialect::Js),
        Some("jsx") => Some(Dialect::Jsx),
        Some("mjs") => Some(Dialect::Mjs),
        Some("cjs") => Some(Dialect::Cjs),
        _ => None,
    }
}

/// `Language` of a dialect: TypeScript dialects are TypeScript, the rest JavaScript.
pub fn language_of(dialect: Dialect) -> Language {
    match dialect {
        Dialect::Ts | Dialect::Tsx | Dialect::Dts => Language::Typescript,
        Dialect::Js | Dialect::Jsx | Dialect::Mjs | Dialect::Cjs => Language::Javascript,
    }
}

/// Stateless and `Sync`; parsers live in thread-local pools.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypeScriptAnalyzer;

impl TypeScriptAnalyzer {
    pub fn new() -> Self {
        Self
    }

    fn analyzer_id() -> AnalyzerId {
        AnalyzerId {
            name: ANALYZER_NAME.to_owned(),
            version: ANALYZER_VERSION,
        }
    }

    /// A unit that only has the module symbol, with the given status and diagnostics.
    fn module_only(
        input: &SourceInput<'_>,
        dialect: Dialect,
        status: ParseStatus,
        diagnostics: Vec<analysis_ir::ParseDiagnostic>,
        stats: UnitStats,
    ) -> ParsedUnit {
        let source = Source::new(input.bytes);
        let module = visit::module_symbol(&source, &module_name(input), false);
        ParsedUnit {
            ir_schema: IR_SCHEMA_VERSION,
            file: input.path.clone(),
            module_path: input.module_path.clone(),
            language: language_of(dialect),
            dialect: Some(dialect),
            content_hash: input.content_hash,
            analyzer: Self::analyzer_id(),
            status,
            symbols: vec![module],
            references: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            framework: Vec::new(),
            facts: Vec::new(),
            diagnostics,
            stats,
        }
    }
}

fn module_name(input: &SourceInput<'_>) -> String {
    input
        .module_path
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or("module")
        .to_owned()
}

impl LanguageAnalyzer for TypeScriptAnalyzer {
    fn id(&self) -> AnalyzerId {
        Self::analyzer_id()
    }

    fn language(&self) -> Language {
        Language::Typescript
    }

    fn supports(&self, path: &RepoPath) -> bool {
        dialect_of(path).is_some()
    }

    fn analyze(
        &self,
        input: &SourceInput<'_>,
        cfg: &AnalyzerConfig,
    ) -> Result<ParsedUnit, AnalyzeError> {
        let started = Instant::now();
        let dialect = dialect_of(&input.path).unwrap_or(Dialect::Ts);
        let source = Source::new(input.bytes);
        let mut stats = UnitStats {
            bytes: input.bytes.len() as u64,
            lines: source.line_count(),
            parse_micros: 0,
        };
        let mut sink = DiagnosticSink::default();

        // Guards: oversize and binary input never reach the parser.
        if input.bytes.len() as u64 > cfg.max_file_bytes {
            sink.note(
                DiagSeverity::Error,
                DiagCode::FileTooLarge,
                "file is larger than the analyzer limit",
                None,
            );
            return Ok(Self::module_only(
                input,
                dialect,
                ParseStatus::Failed {
                    reason: FailReason::TooLarge,
                },
                sink.finish(),
                stats,
            ));
        }
        if input.bytes[..input.bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
            sink.note(
                DiagSeverity::Error,
                DiagCode::UnsupportedConstruct,
                "file looks binary",
                None,
            );
            return Ok(Self::module_only(
                input,
                dialect,
                ParseStatus::Failed {
                    reason: FailReason::Binary,
                },
                sink.finish(),
                stats,
            ));
        }
        if !source.is_valid_utf8() {
            sink.note(
                DiagSeverity::Info,
                DiagCode::UnsupportedConstruct,
                "invalid utf-8 decoded lossily",
                None,
            );
        }

        // Parse with a deadline. The progress callback returns true to cancel.
        let deadline = started + Duration::from_millis(u64::from(cfg.parse_timeout_ms));
        let grammar = Grammar::for_dialect(dialect);
        let bytes = source.bytes;
        let tree = with_parser(grammar, |parser| {
            let mut cancel = |_state: &tree_sitter::ParseState| Instant::now() > deadline;
            let options = ParseOptions::new().progress_callback(&mut cancel);
            let tree = parser.parse_with_options(
                &mut |offset, _| &bytes[offset.min(bytes.len())..],
                None,
                Some(options),
            );
            if tree.is_none() {
                // A cancelled parse would otherwise resume on this parser's next use.
                parser.reset();
            }
            tree
        })?;
        let Some(tree) = tree else {
            sink.note(
                DiagSeverity::Error,
                DiagCode::ParseTimeout,
                "parsing exceeded the time limit",
                None,
            );
            tracing::debug!(reason = "timeout", "parse_failed");
            stats.parse_micros = started.elapsed().as_micros() as u64;
            return Ok(Self::module_only(
                input,
                dialect,
                ParseStatus::Failed {
                    reason: FailReason::Timeout,
                },
                sink.finish(),
                stats,
            ));
        };

        let root = tree.root_node();
        let scan = scan_errors(root, &source, &mut sink);
        let ctx = VisitCtx {
            source,
            module_name: module_name(input),
            error_ranges: &scan.ranges,
            cfg,
        };
        let collected = visit::run(root, &ctx, &mut sink);

        let status = if scan.error_nodes + scan.missing_nodes == 0 {
            ParseStatus::Ok
        } else {
            ParseStatus::Partial {
                error_nodes: scan.error_nodes,
                missing_nodes: scan.missing_nodes,
            }
        };
        stats.parse_micros = started.elapsed().as_micros() as u64;
        let unit = ParsedUnit {
            ir_schema: IR_SCHEMA_VERSION,
            file: input.path.clone(),
            module_path: input.module_path.clone(),
            language: language_of(dialect),
            dialect: Some(dialect),
            content_hash: input.content_hash,
            analyzer: Self::analyzer_id(),
            status,
            symbols: collected.symbols,
            references: collected.references,
            imports: collected.imports,
            exports: collected.exports,
            framework: collected.framework,
            facts: collected.facts,
            diagnostics: sink.finish(),
            stats,
        };
        debug_assert!(
            analysis_ir::validate(&unit).is_ok(),
            "analyzer produced an invalid unit: {:?}",
            analysis_ir::validate(&unit)
        );
        debug_assert!(unit
            .symbols
            .first()
            .is_some_and(|s| s.kind == SymbolKind::Module));
        Ok(unit)
    }
}
