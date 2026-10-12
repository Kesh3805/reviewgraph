//! Benchmark case format, loader and validator (EVAL-001).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use review_core::finding::Severity;
use review_core::ids::SymbolId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseClass {
    CorrectnessRegression,
    SafeChange,
    Security,
    Architecture,
    FpTrap,
    TestGap,
    Concurrency,
    Transaction,
    ApiContract,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrInfo {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationSpec {
    pub path: String,
    pub line: u32,
    #[serde(default)]
    pub window: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    pub id: String,
    pub categories: Vec<String>,
    #[serde(default)]
    pub severity_at_least: Option<Severity>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub location: Option<LocationSpec>,
    #[serde(default)]
    pub claim_any_of: Vec<String>,
    #[serde(default = "default_true")]
    pub must_publish: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub version: u32,
    pub title: String,
    pub classes: Vec<CaseClass>,
    pub language: String,
    #[serde(default)]
    pub frameworks: Vec<String>,
    pub pr: PrInfo,
    pub required_reviewers: Vec<String>,
    #[serde(default)]
    pub expected_findings: Vec<Expectation>,
    #[serde(default)]
    pub forbidden_findings: Vec<Expectation>,
    #[serde(default)]
    pub optional_findings: Vec<Expectation>,
    #[serde(default)]
    pub expect_no_published_findings: bool,
    #[serde(default)]
    pub synthetic_fixtures: bool,
}

impl Case {
    /// Cases with the `example-` prefix are excluded from metrics.
    pub fn is_example(&self) -> bool {
        self.id.starts_with("example-")
    }
}

fn id_is_valid(id: &str) -> bool {
    // ^[a-z]+-[0-9]{3}-[a-z0-9-]+$
    let mut parts = id.splitn(3, '-');
    let (Some(a), Some(b), Some(c)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !a.is_empty()
        && a.bytes().all(|x| x.is_ascii_lowercase())
        && b.len() == 3
        && b.bytes().all(|x| x.is_ascii_digit())
        && !c.is_empty()
        && c.bytes()
            .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'-')
}

/// Parses `case.yaml` text and applies the checks that need no file tree.
pub fn parse_case(yaml: &str) -> Result<Case> {
    let case: Case = serde_yaml::from_str(yaml).context("case.yaml does not match the schema")?;
    validate_case(&case)?;
    Ok(case)
}

fn validate_case(case: &Case) -> Result<()> {
    if case.version != 1 {
        bail!("unsupported case version {}", case.version);
    }
    if !id_is_valid(&case.id) {
        bail!(
            "case id `{}` does not match ^[a-z]+-[0-9]{{3}}-[a-z0-9-]+$",
            case.id
        );
    }
    if case.classes.is_empty() {
        bail!("classes must not be empty");
    }
    let mut seen = BTreeSet::new();
    for e in case
        .expected_findings
        .iter()
        .chain(&case.forbidden_findings)
        .chain(&case.optional_findings)
    {
        if !seen.insert(e.id.as_str()) {
            bail!("duplicate expectation id `{}`", e.id);
        }
        if e.categories.is_empty() {
            bail!("expectation `{}` has no categories", e.id);
        }
        if e.symbol.is_none() && e.location.is_none() && e.claim_any_of.is_empty() {
            bail!(
                "expectation `{}` needs a symbol, a location or claim keywords",
                e.id
            );
        }
        if let Some(sym) = &e.symbol {
            SymbolId::parse(sym)
                .map_err(|err| anyhow!("expectation `{}`: bad symbol id `{sym}`: {err}", e.id))?;
        }
    }
    let safe = case.classes.contains(&CaseClass::SafeChange);
    if case.expect_no_published_findings && !case.expected_findings.is_empty() {
        bail!("expect_no_published_findings requires an empty expected_findings");
    }
    if safe != case.expect_no_published_findings {
        bail!("classes contains safe_change if and only if expect_no_published_findings is true");
    }
    Ok(())
}

/// Loads one case directory: parses, validates, and checks that every location path exists in
/// the head tree (base files plus files added by `head.patch`), or in the base tree.
pub fn load_case(dir: &Path) -> Result<Case> {
    let yaml = fs::read_to_string(dir.join("case.yaml"))
        .with_context(|| format!("reading {}/case.yaml", dir.display()))?;
    let case = parse_case(&yaml)?;
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if dir_name != case.id {
        bail!(
            "case id `{}` does not match directory `{dir_name}`",
            case.id
        );
    }
    if !dir.join("head.patch").is_file() {
        bail!("case `{}` has no head.patch", case.id);
    }
    let patch = fs::read_to_string(dir.join("head.patch")).unwrap_or_default();
    for e in case
        .expected_findings
        .iter()
        .chain(&case.forbidden_findings)
        .chain(&case.optional_findings)
    {
        if let Some(loc) = &e.location {
            let in_base = dir.join("base").join(&loc.path).is_file();
            let in_patch = patch.contains(&format!("+++ b/{}", loc.path));
            if !in_base && !in_patch {
                bail!(
                    "expectation `{}`: location path `{}` not found",
                    e.id,
                    loc.path
                );
            }
        }
    }
    Ok(case)
}

/// Lists case directories (sorted) under `root`.
pub fn list_case_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn hash_dir_into(hasher: &mut blake3::Hasher, root: &Path, dir: &Path) -> Result<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    entries.sort();
    for p in entries {
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        if p.is_dir() {
            hasher.update(rel.as_bytes());
            hasher.update(b"/\0");
            hash_dir_into(hasher, root, &p)?;
        } else {
            hasher.update(rel.as_bytes());
            hasher.update(b"\0");
            hasher.update(&fs::read(&p)?);
            hasher.update(b"\0");
        }
    }
    Ok(())
}

/// blake3 over a directory's relative paths and contents.
pub fn dir_hash(dir: &Path) -> Result<String> {
    let mut h = blake3::Hasher::new();
    hash_dir_into(&mut h, dir, dir)?;
    Ok(h.finalize().to_hex().to_string())
}

/// blake3 over the sorted `(case_id, dir_hash)` pairs.
pub fn corpus_hash(case_dirs: &[PathBuf]) -> Result<String> {
    let mut pairs = Vec::new();
    for d in case_dirs {
        let id = d
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        pairs.push((id, dir_hash(d)?));
    }
    pairs.sort();
    let mut h = blake3::Hasher::new();
    for (id, hash) in pairs {
        h.update(id.as_bytes());
        h.update(b"\0");
        h.update(hash.as_bytes());
        h.update(b"\n");
    }
    Ok(h.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const EXAMPLE: &str = r#"
id: corr-003-missing-await
version: 1
title: t
classes: [correctness_regression]
language: typescript
pr: { title: p, base_branch: main }
required_reviewers: [correctness]
expected_findings:
  - id: E1
    categories: [correctness]
    location: { path: src/a.ts, line: 4, window: 2 }
"#;

    #[test]
    fn case_schema_accepts_example() {
        assert!(parse_case(EXAMPLE).is_ok());
    }

    #[test]
    fn case_rejects_unknown_class() {
        let y = EXAMPLE.replace("correctness_regression", "bogus_class");
        assert!(parse_case(&y).is_err());
    }

    #[test]
    fn case_rejects_bad_symbol_id() {
        let y = EXAMPLE.replace(
            "location: { path: src/a.ts, line: 4, window: 2 }",
            "symbol: \"not a symbol\"",
        );
        assert!(parse_case(&y).is_err());
    }

    #[test]
    fn safe_change_requires_no_expected() {
        let y = EXAMPLE.replace("correctness_regression", "safe_change");
        assert!(parse_case(&y).is_err());
    }

    #[test]
    fn corpus_hash_changes_when_case_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("corr-001-x");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("case.yaml"), "a").unwrap();
        let h1 = corpus_hash(std::slice::from_ref(&d)).unwrap();
        fs::write(d.join("case.yaml"), "b").unwrap();
        let h2 = corpus_hash(std::slice::from_ref(&d)).unwrap();
        assert_ne!(h1, h2);
    }
}
