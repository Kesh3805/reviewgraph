//! Materialises a case into a git repository with deterministic base and head commits
//! (EVAL-001). The work is done by `benchmarks/quality/build.sh`; this module runs it, caches the
//! result under `target/eval-repos/<case_id>-<hash>` and reads back the SHAs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use rayon::prelude::*;

use super::case::{EvalPaths, LoadedCase};

/// The built repository of one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltCase {
    pub case_id: String,
    pub repo: PathBuf,
    pub base_sha: String,
    pub head_sha: String,
}

/// Name of the marker written last by `build.sh`; it holds `base_sha head_sha`.
pub const COMPLETE_MARKER: &str = ".complete";

fn read_marker(dir: &Path) -> Option<(String, String)> {
    let text = fs::read_to_string(dir.join(COMPLETE_MARKER)).ok()?;
    let mut parts = text.split_whitespace();
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

/// Builds `case` into `out_dir` (no caching). Fails with the case id when the patch does not
/// apply; no partial repository is left behind.
pub fn build_into(
    paths: &EvalPaths,
    case: &LoadedCase,
    out_dir: &Path,
) -> anyhow::Result<BuiltCase> {
    let output = Command::new("bash")
        .arg(paths.build_script())
        .arg(&case.dir)
        .arg(out_dir)
        .output()
        .with_context(|| format!("{}: cannot run build.sh", case.case.id))?;
    if !output.status.success() {
        bail!(
            "{}: build failed: {}",
            case.case.id,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let (base_sha, head_sha) = read_marker(out_dir).with_context(|| {
        format!(
            "{}: build.sh wrote no {COMPLETE_MARKER} marker",
            case.case.id
        )
    })?;
    Ok(BuiltCase {
        case_id: case.case.id.clone(),
        repo: out_dir.to_path_buf(),
        base_sha,
        head_sha,
    })
}

/// Builds `case` under `cache_root`, reusing a complete earlier build of the same content.
pub fn build_cached(
    paths: &EvalPaths,
    case: &LoadedCase,
    cache_root: &Path,
) -> anyhow::Result<BuiltCase> {
    let short: String = case.content_hash.chars().take(16).collect();
    let dir = cache_root.join(format!("{}-{short}", case.case.id));
    if let Some((base_sha, head_sha)) = read_marker(&dir) {
        return Ok(BuiltCase {
            case_id: case.case.id.clone(),
            repo: dir,
            base_sha,
            head_sha,
        });
    }
    fs::create_dir_all(cache_root)?;
    build_into(paths, case, &dir)
}

/// Builds every case in parallel into distinct directories.
pub fn build_all(
    paths: &EvalPaths,
    cases: &[LoadedCase],
    cache_root: &Path,
) -> Vec<anyhow::Result<BuiltCase>> {
    cases
        .par_iter()
        .map(|c| build_cached(paths, c, cache_root))
        .collect()
}

/// Parses `expected_shas.txt`: `case_id base_sha head_sha` per line.
pub fn parse_expected_shas(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            Some((
                p.next()?.to_owned(),
                p.next()?.to_owned(),
                p.next()?.to_owned(),
            ))
        })
        .collect()
}
