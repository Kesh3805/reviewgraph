//! The `.review/` state directory: layout, starter config, lock and atomic writes (INIT-011).
//!
//! This module owns every read and write of `.review/` itself, which the walker deliberately
//! skips.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{InitError, InitWarning};
use crate::generated::GeneratedConfig;

pub const REVIEW_DIR: &str = ".review";
pub const FACTS_FILE: &str = "repository.json";

const LAYOUT_DIRS: &[&str] = &[
    "graph/nodes",
    "graph/edges",
    "graph/indexes",
    "graph/metadata",
    "graph/snapshots",
    "ast",
    "symbols",
    "semantic",
    "profile",
    "snapshots",
    "history",
    "cache",
];

const REVIEW_GITIGNORE: &str = "*\n!.gitignore\n!config.yaml\n";

const STARTER_CONFIG: &str =
    "# ReviewGraph repository configuration. This file is yours: init never overwrites it.\n\
version: 1\n\
\n\
# Extra ignore globs, applied on top of .gitignore and .reviewignore.\n\
ignore: []\n\
\n\
# Force files in or out of generated-code classification.\n\
generated:\n\
\x20 include: []\n\
\x20 exclude: []\n\
\n\
reviewers: {}\n\
confidence: {}\n\
rules: []\n";

/// The parts of `.review/config.yaml` that init consumes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReviewConfig {
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub generated: GeneratedSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GeneratedSection {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl ReviewConfig {
    pub fn generated_config(&self) -> GeneratedConfig {
        GeneratedConfig {
            include: self.generated.include.clone(),
            exclude: self.generated.exclude.clone(),
        }
    }
}

pub fn review_dir(root: &Path) -> PathBuf {
    root.join(REVIEW_DIR)
}

/// Reads `.review/config.yaml`. Absent or malformed config yields defaults (the latter with a
/// `config_yaml_parse` warning).
pub fn read_config(root: &Path) -> (ReviewConfig, Vec<InitWarning>) {
    let path = review_dir(root).join("config.yaml");
    let Ok(text) = fs::read_to_string(&path) else {
        return (ReviewConfig::default(), Vec::new());
    };
    match serde_yaml::from_str::<ReviewConfig>(&text) {
        Ok(config) => (config, Vec::new()),
        Err(_) => (
            ReviewConfig::default(),
            vec![InitWarning::new(
                "config_yaml_parse",
                None,
                ".review/config.yaml could not be parsed; defaults were used",
            )],
        ),
    }
}

/// Reads the existing `repository.json`, if any.
pub fn read_facts_json(root: &Path) -> Option<String> {
    fs::read_to_string(review_dir(root).join(FACTS_FILE)).ok()
}

/// Creates the directory layout, `.gitignore` and (only when absent) `config.yaml`.
pub fn ensure_layout(root: &Path) -> Result<PathBuf, InitError> {
    let dir = review_dir(root);
    let denied = |e: std::io::Error| InitError::ReviewDirNotWritable(e.to_string());
    fs::create_dir_all(&dir).map_err(denied)?;
    for sub in LAYOUT_DIRS {
        fs::create_dir_all(dir.join(sub)).map_err(denied)?;
    }
    fs::write(dir.join(".gitignore"), REVIEW_GITIGNORE).map_err(denied)?;
    let config = dir.join("config.yaml");
    // `create_new` never overwrites a user-owned config.
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config)
    {
        Ok(mut file) => file.write_all(STARTER_CONFIG.as_bytes()).map_err(denied)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(denied(e)),
    }
    Ok(dir)
}

/// An exclusive advisory lock on `.review/.lock`, held until dropped (the OS releases it when
/// the file closes).
#[derive(Debug)]
pub struct ReviewLock {
    _file: File,
}

impl ReviewLock {
    /// Takes the lock without blocking. `InitError::Busy` when another holder has it.
    pub fn acquire(dir: &Path) -> Result<Self, InitError> {
        use fs2::FileExt;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".lock"))
            .map_err(|e| InitError::ReviewDirNotWritable(e.to_string()))?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Self { _file: file }),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                Err(InitError::Busy)
            }
            Err(e) => Err(InitError::ReviewDirNotWritable(e.to_string())),
        }
    }
}

/// Writes `contents` to `dir/repository.json` atomically: temp file in the same directory,
/// fsync, rename, fsync of the directory. A failure removes the temp file and leaves any
/// existing `repository.json` untouched.
pub fn write_facts_atomic(dir: &Path, contents: &str) -> Result<PathBuf, InitError> {
    let target = dir.join(FACTS_FILE);
    let io = |e: std::io::Error| InitError::io(dir, e);
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(io)?;
    tmp.write_all(contents.as_bytes()).map_err(io)?;
    tmp.as_file().sync_all().map_err(io)?;
    tmp.persist(&target)
        .map_err(|e| InitError::io(dir, e.error))?;
    #[cfg(unix)]
    {
        if let Ok(handle) = File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    Ok(target)
}

/// Writes a derived file under `.review/cache/` atomically.
pub fn write_cache_file(dir: &Path, name: &str, contents: &str) -> Result<PathBuf, InitError> {
    let cache = dir.join("cache");
    fs::create_dir_all(&cache).map_err(|e| InitError::io(&cache, e))?;
    let io = |e: std::io::Error| InitError::io(&cache, e);
    let mut tmp = tempfile::NamedTempFile::new_in(&cache).map_err(io)?;
    tmp.write_all(contents.as_bytes()).map_err(io)?;
    tmp.as_file().sync_all().map_err(io)?;
    let target = cache.join(name);
    tmp.persist(&target)
        .map_err(|e| InitError::io(&cache, e.error))?;
    Ok(target)
}

/// Raw bytes of `.review/config.yaml`, if present.
pub fn read_config_raw(root: &Path) -> Option<Vec<u8>> {
    fs::read(review_dir(root).join("config.yaml")).ok()
}
