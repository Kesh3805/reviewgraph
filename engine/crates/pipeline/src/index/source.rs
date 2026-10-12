//! Where the files of an index run come from (IDX-001 step 2).
//!
//! Only a working-tree source exists today: a checkout (or a mirror materialized by the
//! repository crate) that `repository::walk` can enumerate. Symlinks are never followed, files
//! above the size cap are reported as skipped, and binary files are detected by the walker's
//! NUL sniff, so none of them ever reach an analyzer.

use std::path::{Path, PathBuf};

use analysis_ir::AnalyzerRegistry;
use repository::generated::{classify_generated, GeneratedConfig, GitAttributesView};
use repository::read::BoundedReader;
use repository::tsconfig::detect_tsconfigs;
use repository::walk::{walk, FileClass, WalkOptions};
use review_core::location::RepoPath;

use super::config::IndexConfig;
use super::error::IndexError;

/// The files an index run reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpec {
    /// A directory on disk, treated as the repository root.
    WorkingTree { root: PathBuf },
}

impl SourceSpec {
    pub fn root(&self) -> &Path {
        match self {
            Self::WorkingTree { root } => root,
        }
    }
}

/// One analyzable file found by the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: RepoPath,
    pub size: u64,
    pub is_generated: bool,
}

/// Result of [`walk_source`]: what to parse and what was left out.
#[derive(Debug, Clone, Default)]
pub struct WalkedSource {
    /// Canonical root of the walk.
    pub root: PathBuf,
    /// Files an analyzer supports, sorted by path.
    pub files: Vec<SourceFile>,
    /// Supported files above the size cap.
    pub skipped_too_large: Vec<RepoPath>,
    /// Supported files the walker classified as binary.
    pub skipped_binary: Vec<RepoPath>,
    /// Canonical serialization of the tsconfig set, hashed into the config fingerprint.
    pub tsconfig_canonical: Vec<u8>,
    /// Every inventory path, for the resolver (sorted).
    pub all_paths: Vec<RepoPath>,
}

/// Enumerates `spec` and keeps the files `analyzers` support.
pub fn walk_source(
    spec: &SourceSpec,
    cfg: &IndexConfig,
    analyzers: &AnalyzerRegistry,
) -> Result<WalkedSource, IndexError> {
    let opts = WalkOptions {
        max_analyze_bytes: cfg.max_file_bytes,
        ..WalkOptions::default()
    };
    let (inventory, _warnings) =
        walk(spec.root(), &opts).map_err(|e| IndexError::Walk(e.to_string()))?;
    let reader = BoundedReader::new(inventory.root.clone());
    let (tsconfigs, _) = detect_tsconfigs(&inventory, &reader);
    let attrs = GitAttributesView::from_inventory(&inventory, &reader);
    let (generated, _) = classify_generated(
        &inventory,
        &tsconfigs,
        &attrs,
        &GeneratedConfig::default(),
        &reader,
    );

    let mut out = WalkedSource {
        root: inventory.root.clone(),
        tsconfig_canonical: serde_json::to_vec(&tsconfigs).unwrap_or_default(),
        ..WalkedSource::default()
    };
    for entry in &inventory.entries {
        out.all_paths.push(entry.path.clone());
        if analyzers.for_path(&entry.path).is_none() {
            continue;
        }
        match entry.class {
            FileClass::Source => out.files.push(SourceFile {
                path: entry.path.clone(),
                size: entry.size,
                is_generated: generated.is_generated(&entry.path),
            }),
            FileClass::TooLarge => out.skipped_too_large.push(entry.path.clone()),
            FileClass::Binary => out.skipped_binary.push(entry.path.clone()),
            FileClass::LfsPointer | FileClass::Sensitive | FileClass::Symlink => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn registry() -> AnalyzerRegistry {
        let mut r = AnalyzerRegistry::new();
        r.register(Arc::new(lang_typescript::TypeScriptAnalyzer::new()));
        r
    }

    #[test]
    fn walk_keeps_supported_files_and_reports_oversized_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.ts"), "export const a = 1;\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
        std::fs::write(dir.path().join("src/big.ts"), vec![b'a'; 4096]).unwrap();
        std::fs::write(dir.path().join("src/bin.ts"), b"export\0const").unwrap();
        let cfg = IndexConfig {
            max_file_bytes: 1024,
            ..IndexConfig::default()
        };
        let walked = walk_source(
            &SourceSpec::WorkingTree {
                root: dir.path().to_path_buf(),
            },
            &cfg,
            &registry(),
        )
        .unwrap();
        let kept: Vec<&str> = walked.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(kept, vec!["src/a.ts"]);
        assert_eq!(walked.skipped_too_large.len(), 1);
        assert_eq!(walked.skipped_binary.len(), 1);
    }
}
