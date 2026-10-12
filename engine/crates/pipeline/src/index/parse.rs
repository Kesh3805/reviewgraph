//! Read + parse step of a full index (IDX-001, IDX-003).
//!
//! Every function here is synchronous and CPU-bound: callers run it on the dedicated rayon pool
//! inside `spawn_blocking`. File bytes are read per file inside the worker, hashed, parsed and
//! dropped; only the `ParsedUnit` survives. Analyzer panics are contained per file and never
//! store the panic payload (it could contain source text).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;

use analysis_ir::unit::FailReason as IrFailReason;
use analysis_ir::{AnalyzerConfig, AnalyzerRegistry, ParseStatus, ParsedUnit, SourceInput};
use rayon::prelude::*;
use review_core::language::Language;
use review_core::location::ContentHash;
use review_core::symbol::ModulePath;

use super::config::IndexConfig;
use super::parse_cache::ParseCacheKey;
use super::source::SourceFile;
use super::tolerance::FileOutcomeKind;

/// Why a file failed to parse. No variant carries a message: paths and codes only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    AnalyzerError,
    Panic,
    Timeout,
    Utf8,
}

impl FailReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnalyzerError => "analyzer_error",
            Self::Panic => "panic",
            Self::Timeout => "timeout",
            Self::Utf8 => "utf8",
        }
    }
}

/// Why a file was not parsed at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    TooLarge,
    Binary,
    Unsupported,
    Excluded,
}

impl SkipReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::Binary => "binary",
            Self::Unsupported => "unsupported",
            Self::Excluded => "excluded",
        }
    }
}

/// What happened to one file. `Failed` keeps the module-only unit the analyzer returned (so the
/// file still gets a file node and a module symbol); a panic or an unreadable file has none.
#[derive(Debug, Clone)]
pub enum FileOutcome {
    Ok(Arc<ParsedUnit>),
    /// Error nodes present; symbols are still emitted and linked.
    Partial(Arc<ParsedUnit>),
    Failed {
        reason: FailReason,
        unit: Option<Arc<ParsedUnit>>,
    },
    Skipped {
        reason: SkipReason,
    },
}

impl FileOutcome {
    pub fn kind(&self) -> FileOutcomeKind {
        match self {
            Self::Ok(_) => FileOutcomeKind::Ok,
            Self::Partial(_) => FileOutcomeKind::Partial,
            Self::Failed { .. } => FileOutcomeKind::Failed,
            Self::Skipped { .. } => FileOutcomeKind::Skipped,
        }
    }

    /// The unit to link, when there is one.
    pub fn unit(&self) -> Option<&Arc<ParsedUnit>> {
        match self {
            Self::Ok(u) | Self::Partial(u) => Some(u),
            Self::Failed { unit, .. } => unit.as_ref(),
            Self::Skipped { .. } => None,
        }
    }
}

/// One file after the parse step.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub file: SourceFile,
    pub hash: ContentHash,
    pub language: Language,
    pub analyzer_name: String,
    pub analyzer_version: String,
    pub outcome: FileOutcome,
    /// The unit came from the parse cache; no analyzer ran.
    pub from_cache: bool,
}

impl ParsedFile {
    pub fn cache_key(&self) -> ParseCacheKey {
        ParseCacheKey {
            path: self.file.path.as_str().to_owned(),
            content_hash: *self.hash.as_bytes(),
            analyzer_version: self.analyzer_version.clone(),
        }
    }
}

fn analyzer_config(cfg: &IndexConfig) -> AnalyzerConfig {
    AnalyzerConfig {
        max_file_bytes: cfg.max_file_bytes,
        parse_timeout_ms: u32::try_from(cfg.parse_timeout.as_millis()).unwrap_or(u32::MAX),
        ..AnalyzerConfig::default()
    }
}

fn read_bounded(root: &Path, file: &SourceFile, cfg: &IndexConfig) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let handle = std::fs::File::open(root.join(file.path.as_str()))?;
    let mut buf = Vec::new();
    handle
        .take(cfg.max_file_bytes.saturating_add(1))
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Content hashes of `files`, for batched cache lookups (`None` = unreadable). Runs on the
/// calling thread's rayon pool, results in input order.
pub fn hash_files(
    root: &Path,
    files: &[SourceFile],
    cfg: &IndexConfig,
) -> Vec<Option<ContentHash>> {
    files
        .par_iter()
        .map(|file| {
            read_bounded(root, file, cfg)
                .ok()
                .map(|bytes| ContentHash::of(&bytes))
        })
        .collect()
}

/// Parses `files` in parallel, results in input order. `cached[i]` is a ready unit for
/// `files[i]` (a cache hit) that skips the analyzer; pass an empty slice for "no cache".
pub fn parse_files(
    root: &Path,
    files: &[SourceFile],
    analyzers: &AnalyzerRegistry,
    cfg: &IndexConfig,
    cached: &[Option<Arc<ParsedUnit>>],
) -> Vec<ParsedFile> {
    let acfg = analyzer_config(cfg);
    files
        .par_iter()
        .enumerate()
        .map(|(i, file)| {
            let hit = cached.get(i).and_then(Clone::clone);
            parse_one(root, file, analyzers, cfg, &acfg, hit)
        })
        .collect()
}

fn parse_one(
    root: &Path,
    file: &SourceFile,
    analyzers: &AnalyzerRegistry,
    cfg: &IndexConfig,
    acfg: &AnalyzerConfig,
    hit: Option<Arc<ParsedUnit>>,
) -> ParsedFile {
    let Some(analyzer) = analyzers.for_path(&file.path) else {
        return ParsedFile {
            file: file.clone(),
            hash: ContentHash::from_bytes([0u8; 32]),
            language: Language::Other,
            analyzer_name: String::new(),
            analyzer_version: String::new(),
            outcome: FileOutcome::Skipped {
                reason: SkipReason::Unsupported,
            },
            from_cache: false,
        };
    };
    let id = analyzer.id();
    let mut parsed = ParsedFile {
        file: file.clone(),
        hash: ContentHash::from_bytes([0u8; 32]),
        language: analyzer.language(),
        analyzer_name: id.name,
        analyzer_version: id.version.to_string(),
        outcome: FileOutcome::Failed {
            reason: FailReason::AnalyzerError,
            unit: None,
        },
        from_cache: false,
    };

    if let Some(unit) = hit {
        parsed.hash = unit.content_hash;
        parsed.language = unit.language;
        parsed.outcome = classify(unit);
        parsed.from_cache = true;
        return parsed;
    }

    let bytes = match read_bounded(root, file, cfg) {
        Ok(bytes) => bytes,
        Err(_) => return parsed,
    };
    parsed.hash = ContentHash::of(&bytes);
    if bytes.len() as u64 > cfg.max_file_bytes {
        parsed.outcome = FileOutcome::Skipped {
            reason: SkipReason::TooLarge,
        };
        return parsed;
    }
    let input = SourceInput {
        path: file.path.clone(),
        module_path: ModulePath::of(&file.path),
        bytes: &bytes,
        content_hash: parsed.hash,
        is_generated: file.is_generated,
    };
    let result = catch_unwind(AssertUnwindSafe(|| analyzer.analyze(&input, acfg)));
    parsed.outcome = match result {
        Ok(Ok(unit)) => classify(Arc::new(unit)),
        Ok(Err(_)) => FileOutcome::Failed {
            reason: FailReason::AnalyzerError,
            unit: None,
        },
        Err(_) => FileOutcome::Failed {
            reason: FailReason::Panic,
            unit: None,
        },
    };
    parsed
}

/// Maps the analyzer's own status onto the outcome classes of the tolerance policy.
fn classify(unit: Arc<ParsedUnit>) -> FileOutcome {
    match unit.status {
        ParseStatus::Ok => FileOutcome::Ok(unit),
        ParseStatus::Partial { .. } => FileOutcome::Partial(unit),
        ParseStatus::Failed { reason } => match reason {
            IrFailReason::TooLarge => FileOutcome::Skipped {
                reason: SkipReason::TooLarge,
            },
            IrFailReason::Binary => FileOutcome::Skipped {
                reason: SkipReason::Binary,
            },
            IrFailReason::Timeout => FileOutcome::Failed {
                reason: FailReason::Timeout,
                unit: Some(unit),
            },
            IrFailReason::NotUtf8Decodable => FileOutcome::Failed {
                reason: FailReason::Utf8,
                unit: Some(unit),
            },
            IrFailReason::Internal => FileOutcome::Failed {
                reason: FailReason::AnalyzerError,
                unit: Some(unit),
            },
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use analysis_ir::traits::{AnalyzeError, LanguageAnalyzer};
    use analysis_ir::AnalyzerId;
    use review_core::location::RepoPath;

    fn registry() -> AnalyzerRegistry {
        let mut r = AnalyzerRegistry::new();
        r.register(Arc::new(lang_typescript::TypeScriptAnalyzer::new()));
        r
    }

    fn write(dir: &Path, rel: &str, body: &str) -> SourceFile {
        let full = dir.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, body).unwrap();
        SourceFile {
            path: RepoPath::new(rel).unwrap(),
            size: body.len() as u64,
            is_generated: false,
        }
    }

    #[test]
    fn clean_and_broken_files_get_ok_and_partial() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            write(dir.path(), "a.ts", "export const a = 1;\n"),
            write(dir.path(), "b.ts", "export function ( {{{ \n"),
        ];
        let out = parse_files(
            dir.path(),
            &files,
            &registry(),
            &IndexConfig::default(),
            &[],
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].outcome.kind(), FileOutcomeKind::Ok);
        assert_eq!(out[1].outcome.kind(), FileOutcomeKind::Partial);
        assert!(out[1].outcome.unit().is_some());
    }

    #[derive(Debug)]
    struct PanickingAnalyzer;

    impl LanguageAnalyzer for PanickingAnalyzer {
        fn id(&self) -> AnalyzerId {
            AnalyzerId {
                name: "panicky".to_owned(),
                version: "1.0.0".parse().unwrap(),
            }
        }
        fn language(&self) -> Language {
            Language::Typescript
        }
        fn supports(&self, path: &RepoPath) -> bool {
            path.as_str().ends_with(".ts")
        }
        #[allow(clippy::panic)]
        fn analyze(
            &self,
            _input: &SourceInput<'_>,
            _cfg: &AnalyzerConfig,
        ) -> Result<ParsedUnit, AnalyzeError> {
            panic!("boom with secret source text");
        }
    }

    #[test]
    fn analyzer_panic_is_contained_and_file_failed() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![write(dir.path(), "a.ts", "export const a = 1;\n")];
        let mut registry = AnalyzerRegistry::new();
        registry.register(Arc::new(PanickingAnalyzer));
        let out = parse_files(dir.path(), &files, &registry, &IndexConfig::default(), &[]);
        match &out[0].outcome {
            FileOutcome::Failed { reason, unit } => {
                assert_eq!(*reason, FailReason::Panic);
                assert!(unit.is_none());
            }
            other => unreachable!("expected a failed file, got {other:?}"),
        }
    }

    #[test]
    fn cached_units_skip_the_analyzer() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![write(dir.path(), "a.ts", "export const a = 1;\n")];
        let first = parse_files(
            dir.path(),
            &files,
            &registry(),
            &IndexConfig::default(),
            &[],
        );
        let unit = first[0].outcome.unit().cloned();
        assert!(unit.is_some());
        let again = parse_files(
            dir.path(),
            &files,
            &registry(),
            &IndexConfig::default(),
            &[unit],
        );
        assert!(again[0].from_cache);
        assert_eq!(again[0].hash, first[0].hash);
    }

    #[test]
    fn hashes_follow_input_order() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            write(dir.path(), "a.ts", "a"),
            write(dir.path(), "b.ts", "b"),
        ];
        let hashes = hash_files(dir.path(), &files, &IndexConfig::default());
        assert_eq!(hashes[0], Some(ContentHash::of(b"a")));
        assert_eq!(hashes[1], Some(ContentHash::of(b"b")));
    }
}
