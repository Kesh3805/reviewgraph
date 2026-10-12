//! Shared helpers for source-scanning invariant tests (std only).
#![allow(dead_code)]

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

pub type TestResult = Result<(), Box<dyn Error>>;

/// The `engine/` directory (the Cargo workspace root).
pub fn engine_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.rs` file under `dir`, recursively, sorted for deterministic output.
pub fn rust_files(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if !d.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&d)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// `src/` of a crate under `engine/crates/`.
pub fn crate_src(name: &str) -> PathBuf {
    engine_root().join("crates").join(name).join("src")
}

/// Path relative to `engine/`, with forward slashes (the allowlist format).
pub fn rel(path: &Path) -> String {
    let root = engine_root();
    let root = fs::canonicalize(&root).unwrap_or(root);
    let full = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    full.strip_prefix(&root)
        .unwrap_or(&full)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Allowlist lines: `path | reason`. Returns `(path, reason)`; comments and blanks are skipped.
pub fn read_allowlist(file: &str) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("allowlists")
        .join(file);
    let text = fs::read_to_string(path)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once('|') {
            Some((p, r)) => (p.trim().to_owned(), r.trim().to_owned()),
            None => (l.to_owned(), String::new()),
        })
        .collect())
}

/// Scans `.rs` files under each crate's `src/` for case-insensitive `tokens`, skipping
/// allowlisted files. Returns `file:line: token` (never the source line itself).
pub fn scan_crates(
    crates: &[&str],
    tokens: &[&str],
    allowlisted: &[String],
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut hits = Vec::new();
    for name in crates {
        for file in rust_files(&crate_src(name))? {
            let relative = rel(&file);
            if allowlisted.contains(&relative) {
                continue;
            }
            let text = fs::read_to_string(&file)?;
            for (idx, line) in text.lines().enumerate() {
                let lower = line.to_lowercase();
                for token in tokens {
                    if lower.contains(&token.to_lowercase()) {
                        hits.push(format!("{relative}:{}: {token}", idx + 1));
                    }
                }
            }
        }
    }
    Ok(hits)
}
