//! `PythonAnalyzer`: tree-sitter based analysis of Python files (ADR-006, ADR-007). Source is
//! parsed, never executed or imported.

use std::time::{Duration, Instant};

use analysis_ir::diagnostic::{DiagCode, DiagSeverity};
use analysis_ir::{
    AnalyzeError, AnalyzerConfig, AnalyzerId, FailReason, IrSymbol, LanguageAnalyzer, ParseStatus,
    ParsedUnit, SourceInput, UnitStats, IR_SCHEMA_VERSION,
};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::symbol::SymbolKind;
use tree_sitter::ParseOptions;

use crate::adapters;
use crate::diagnostics::{scan_errors, DiagnosticSink};
use crate::facts;
use crate::hashing::{self, child_placeholder, is_container, node_tokens, Fold};
use crate::imports;
use crate::ordinals::assign_ordinals;
use crate::parser_pool::with_parser;
use crate::symbols::{self, Table};
use crate::text::Source;
use crate::{ANALYZER_NAME, ANALYZER_VERSION};

/// Bytes inspected for a NUL, which marks binary content.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Directories whose files are never analyzed: virtual environments and caches.
const EXCLUDED_SEGMENTS: &[&str] = &[
    "site-packages",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
];

/// Stateless and `Sync`; the parser lives in a thread-local pool.
#[derive(Debug, Clone, Copy, Default)]
pub struct PythonAnalyzer;

impl PythonAnalyzer {
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
        status: ParseStatus,
        diagnostics: Vec<analysis_ir::ParseDiagnostic>,
        stats: UnitStats,
    ) -> ParsedUnit {
        let source = Source::new(input.bytes);
        let module = module_symbol(&source, &module_name(input), false);
        ParsedUnit {
            ir_schema: IR_SCHEMA_VERSION,
            file: input.path.clone(),
            module_path: input.module_path.clone(),
            language: Language::Python,
            dialect: None,
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

/// The module name: the file stem, or the package directory for `__init__.py`.
fn module_name(input: &SourceInput<'_>) -> String {
    let mut segments = input.module_path.as_str().rsplit('/');
    let last = segments.next().unwrap_or("module");
    if last == "__init__" {
        return segments.next().unwrap_or("__init__").to_owned();
    }
    last.to_owned()
}

/// `symbols[0]`: the module symbol covering the whole file.
fn module_symbol(source: &Source<'_>, name: &str, has_errors: bool) -> IrSymbol {
    let mut module = IrSymbol::new(
        analysis_ir::LocalId(0),
        SymbolKind::Module,
        name,
        vec!["__module__".to_owned()],
        source.whole(),
    );
    module.has_errors = has_errors;
    module
}

/// Fills the hash fields of every symbol. A container's body hash folds each member into one
/// placeholder token.
fn hash_symbols(table: &mut Table<'_>, source: &Source<'_>) {
    let mut folds: Vec<Vec<Fold>> = vec![Vec::new(); table.symbols.len()];
    for (index, symbol) in table.symbols.iter().enumerate().skip(1) {
        if let Some(parent) = symbol.parent {
            let outer = table.entries[index].outer;
            folds[parent.0 as usize].push(Fold {
                start: outer.start_byte(),
                end: outer.end_byte(),
                token: child_placeholder(symbol.kind.as_id_str(), &symbol.name),
            });
        }
    }
    let Table { symbols, entries } = table;
    for ((symbol, entry), fold) in symbols.iter_mut().zip(entries.iter()).zip(folds.iter()) {
        let tokens = match entry.body {
            Some(node) if is_container(symbol.kind) => node_tokens(node, source, fold),
            Some(node) => node_tokens(node, source, &[]),
            None => Vec::new(),
        };
        hashing::assign(symbol, &tokens);
    }
}

impl LanguageAnalyzer for PythonAnalyzer {
    fn id(&self) -> AnalyzerId {
        Self::analyzer_id()
    }

    fn language(&self) -> Language {
        Language::Python
    }

    fn supports(&self, path: &RepoPath) -> bool {
        let lower = path.as_str().to_ascii_lowercase();
        if !(lower.ends_with(".py") || lower.ends_with(".pyi")) {
            return false;
        }
        !path
            .as_str()
            .split('/')
            .any(|segment| EXCLUDED_SEGMENTS.contains(&segment))
    }

    fn analyze(
        &self,
        input: &SourceInput<'_>,
        cfg: &AnalyzerConfig,
    ) -> Result<ParsedUnit, AnalyzeError> {
        let started = Instant::now();
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
        let bytes = source.bytes;
        let tree = with_parser(|parser| {
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
                ParseStatus::Failed {
                    reason: FailReason::Timeout,
                },
                sink.finish(),
                stats,
            ));
        };

        let root = tree.root_node();
        let scan = scan_errors(root, &source, &mut sink);
        let module = module_symbol(&source, &module_name(input), !scan.ranges.is_empty());
        let mut table = symbols::collect(root, source, &mut sink, module);
        symbols::mark_errors(&mut table.symbols, &scan.ranges);
        let report = assign_ordinals(&mut table.symbols);
        if report.dropped > 0 {
            sink.note(
                DiagSeverity::Error,
                DiagCode::DuplicateSymbol,
                "too many duplicate symbols; some were dropped",
                None,
            );
        }
        for _ in &report.duplicate_groups {
            sink.note(
                DiagSeverity::Info,
                DiagCode::DuplicateSymbol,
                "duplicate symbol name in one scope",
                None,
            );
        }
        hash_symbols(&mut table, &source);

        let is_init = input.path.as_str().ends_with("__init__.py");
        let module_facts = imports::collect(root, &source, &table.symbols, is_init, &mut sink);
        let walked = facts::collect(
            root,
            source,
            &table,
            &module_facts.bindings,
            cfg,
            adapters::is_test_path(input.path.as_str()),
            &mut sink,
        );
        let framework = adapters::run(
            input.path.as_str(),
            &table.symbols,
            &module_facts.imports,
            &module_facts.bindings,
            cfg,
        );

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
            language: Language::Python,
            dialect: None,
            content_hash: input.content_hash,
            analyzer: Self::analyzer_id(),
            status,
            symbols: table.symbols,
            references: walked.references,
            imports: module_facts.imports,
            exports: module_facts.exports,
            framework,
            facts: walked.facts,
            diagnostics: sink.finish(),
            stats,
        };
        debug_assert!(
            analysis_ir::validate(&unit).is_ok(),
            "analyzer produced an invalid unit: {:?}",
            analysis_ir::validate(&unit)
        );
        Ok(unit)
    }
}
