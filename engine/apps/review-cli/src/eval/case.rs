//! Benchmark case format, loader and validator (EVAL-001).
//!
//! Layout: `benchmarks/quality/cases/<case_id>/{case.yaml, base/**, head.patch,
//! review-config.yaml?, context.json?}`. The shared template `benchmarks/quality/templates/nest-app/`
//! is copied under every case's `base/`. `context.json` is the hand-authored reviewer context
//! (a stand-in until the context engine, CTX-*, builds it from the repository).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use model_gateway::{OutputSchema, SchemaValidators};
use review_core::finding::Severity;
use review_core::ids::SymbolId;
use reviewers::ContextSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Case classes (the closed EVAL-001 enum).
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

impl CaseClass {
    pub const ALL: [CaseClass; 9] = [
        Self::CorrectnessRegression,
        Self::SafeChange,
        Self::Security,
        Self::Architecture,
        Self::FpTrap,
        Self::TestGap,
        Self::Concurrency,
        Self::Transaction,
        Self::ApiContract,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CorrectnessRegression => "correctness_regression",
            Self::SafeChange => "safe_change",
            Self::Security => "security",
            Self::Architecture => "architecture",
            Self::FpTrap => "fp_trap",
            Self::TestGap => "test_gap",
            Self::Concurrency => "concurrency",
            Self::Transaction => "transaction",
            Self::ApiContract => "api_contract",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrInfo {
    pub title: String,
    pub description: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub path: String,
    pub line: u32,
    #[serde(default = "default_window")]
    pub window: u32,
}

const fn default_window() -> u32 {
    3
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedFinding {
    pub id: String,
    pub categories: Vec<String>,
    #[serde(default)]
    pub severity_at_least: Option<Severity>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub claim_any_of: Vec<String>,
    /// `false`: being at least internal (verified, not published) is enough.
    #[serde(default = "yes")]
    pub must_publish: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenFinding {
    pub id: String,
    pub categories: Vec<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub claim_any_of: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionalFinding {
    pub id: String,
    pub categories: Vec<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub location: Option<Location>,
}

/// `case.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub version: u32,
    pub title: String,
    pub classes: Vec<CaseClass>,
    pub language: String,
    pub frameworks: Vec<String>,
    pub pr: PrInfo,
    pub required_reviewers: Vec<String>,
    pub expected_findings: Vec<ExpectedFinding>,
    pub forbidden_findings: Vec<ForbiddenFinding>,
    pub optional_findings: Vec<OptionalFinding>,
    pub expect_no_published_findings: bool,
    pub synthetic_fixtures: bool,
}

impl Case {
    pub fn has_class(&self, c: CaseClass) -> bool {
        self.classes.contains(&c)
    }

    /// Example cases (`example-*`) are validated but excluded from metrics.
    pub fn is_example(&self) -> bool {
        self.id.starts_with("example-")
    }
}

/// A case with its directory and optional reviewer context.
#[derive(Debug, Clone)]
pub struct LoadedCase {
    pub case: Case,
    pub dir: PathBuf,
    pub context: Option<ContextSnapshot>,
    /// blake3 over the directory contents.
    pub content_hash: String,
}

/// Where the corpus lives.
#[derive(Debug, Clone)]
pub struct EvalPaths {
    /// Repository root (contains `benchmarks/` and `fixtures/`).
    pub root: PathBuf,
}

impl EvalPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Walks up from `start` to the first directory with `benchmarks/quality`.
    pub fn discover(start: &Path) -> anyhow::Result<Self> {
        let mut dir = Some(start);
        while let Some(d) = dir {
            if d.join("benchmarks").join("quality").is_dir() {
                return Ok(Self::new(d));
            }
            dir = d.parent();
        }
        bail!("no benchmarks/quality directory above {}", start.display())
    }

    pub fn quality(&self) -> PathBuf {
        self.root.join("benchmarks").join("quality")
    }

    pub fn cases(&self) -> PathBuf {
        self.quality().join("cases")
    }

    pub fn template(&self) -> PathBuf {
        self.quality().join("templates").join("nest-app")
    }

    pub fn schema(&self) -> PathBuf {
        self.quality().join("schema").join("case.v1.schema.json")
    }

    pub fn fixtures(&self) -> PathBuf {
        self.root.join("fixtures").join("model-replay")
    }

    pub fn build_script(&self) -> PathBuf {
        self.quality().join("build.sh")
    }
}

/// Every file under `dir` as sorted forward-slash relative paths.
pub fn list_files(dir: &Path) -> anyhow::Result<Vec<String>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> anyhow::Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// blake3 over `(relative path, bytes)` of every file, in sorted order. Line endings are
/// normalised to LF so the hash is the same on every checkout.
pub fn dir_hash(dir: &Path) -> anyhow::Result<String> {
    let mut h = blake3::Hasher::new();
    for rel in list_files(dir)? {
        let bytes = fs::read(dir.join(&rel))?;
        let text = bytes
            .iter()
            .copied()
            .filter(|b| *b != b'\r')
            .collect::<Vec<u8>>();
        h.update(&(rel.len() as u64).to_le_bytes());
        h.update(rel.as_bytes());
        h.update(&(text.len() as u64).to_le_bytes());
        h.update(&text);
    }
    Ok(h.finalize().to_hex().to_string())
}

/// `corpus_hash = blake3(sorted (case_id, blake3(dir)))`, including the shared template.
pub fn corpus_hash(paths: &EvalPaths, cases: &[LoadedCase]) -> anyhow::Result<String> {
    let mut pairs: Vec<(String, String)> = cases
        .iter()
        .map(|c| (c.case.id.clone(), c.content_hash.clone()))
        .collect();
    pairs.push(("__template__".into(), dir_hash(&paths.template())?));
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

/// Paths a unified diff creates or modifies in the head tree.
pub fn patch_paths(patch: &str) -> BTreeSet<String> {
    patch
        .lines()
        .filter_map(|l| l.strip_prefix("+++ b/"))
        .map(|p| p.trim().to_owned())
        .collect()
}

/// Paths a unified diff deletes.
pub fn patch_deleted(patch: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut last_old: Option<String> = None;
    for line in patch.lines() {
        if let Some(p) = line.strip_prefix("--- a/") {
            last_old = Some(p.trim().to_owned());
        } else if line.starts_with("+++ /dev/null") {
            if let Some(p) = last_old.take() {
                out.insert(p);
            }
        }
    }
    out
}

/// Validates cases against the schema and the EVAL-001 rules.
#[derive(Debug)]
pub struct CaseValidator {
    schema: OutputSchema,
    validators: SchemaValidators,
    template_files: BTreeSet<String>,
}

fn id_regex_ok(id: &str) -> bool {
    let parts: Vec<&str> = id.splitn(3, '-').collect();
    parts.len() == 3
        && !parts[0].is_empty()
        && parts[0].bytes().all(|b| b.is_ascii_lowercase())
        && parts[1].len() == 3
        && parts[1].bytes().all(|b| b.is_ascii_digit())
        && !parts[2].is_empty()
        && parts[2]
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl CaseValidator {
    pub fn new(paths: &EvalPaths) -> anyhow::Result<Self> {
        let text = fs::read_to_string(paths.schema())
            .with_context(|| format!("reading {}", paths.schema().display()))?;
        let value: Value = serde_json::from_str(&text)?;
        let schema = OutputSchema::new("case", "v1", value);
        let validators = SchemaValidators::new();
        validators
            .compile(&schema)
            .map_err(|e| anyhow::anyhow!("case schema does not compile: {e}"))?;
        Ok(Self {
            schema,
            validators,
            template_files: list_files(&paths.template())?.into_iter().collect(),
        })
    }

    /// Loads and validates one case directory. Every problem is reported.
    pub fn load(&self, dir: &Path) -> Result<LoadedCase, Vec<String>> {
        let mut errors = Vec::new();
        let dir_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let yaml = match fs::read_to_string(dir.join("case.yaml")) {
            Ok(t) => t,
            Err(e) => return Err(vec![format!("{dir_name}: cannot read case.yaml: {e}")]),
        };
        let value: Value = match serde_yaml::from_str(&yaml) {
            Ok(v) => v,
            Err(e) => return Err(vec![format!("{dir_name}: case.yaml is not YAML: {e}")]),
        };
        match self.validators.validate(&self.schema, &value) {
            Ok(errs) if errs.is_empty() => {}
            Ok(errs) => {
                for e in errs {
                    errors.push(format!(
                        "{dir_name}: schema: {} ({}): {}",
                        e.instance_path, e.keyword, e.message
                    ));
                }
                return Err(errors);
            }
            Err(e) => return Err(vec![format!("{dir_name}: schema: {e}")]),
        }
        let case: Case = match serde_json::from_value(value) {
            Ok(c) => c,
            Err(e) => return Err(vec![format!("{dir_name}: {e}")]),
        };
        if case.id != dir_name {
            errors.push(format!(
                "{dir_name}: id `{}` differs from the directory name",
                case.id
            ));
        }
        if !id_regex_ok(&case.id) {
            errors.push(format!(
                "{dir_name}: id does not match ^[a-z]+-[0-9]{{3}}-[a-z0-9-]+$"
            ));
        }

        let patch = fs::read_to_string(dir.join("head.patch")).unwrap_or_default();
        if !dir.join("head.patch").is_file() {
            errors.push(format!("{dir_name}: head.patch is missing"));
        }
        let mut base_files: BTreeSet<String> = self.template_files.clone();
        base_files.extend(list_files(&dir.join("base")).unwrap_or_default());
        let deleted = patch_deleted(&patch);
        let mut head_files: BTreeSet<String> = base_files.difference(&deleted).cloned().collect();
        head_files.extend(patch_paths(&patch));

        let mut ids = BTreeSet::new();
        let mut check_target = |id: &str,
                                symbol: &Option<String>,
                                location: &Option<Location>,
                                errors: &mut Vec<String>| {
            if !ids.insert(id.to_owned()) {
                errors.push(format!("{dir_name}: duplicate expectation id {id}"));
            }
            if let Some(s) = symbol {
                if let Err(e) = SymbolId::parse(s) {
                    errors.push(format!("{dir_name}: {id}: bad symbol id `{s}`: {e}"));
                }
            }
            if let Some(l) = location {
                if !head_files.contains(&l.path) && !base_files.contains(&l.path) {
                    errors.push(format!(
                        "{dir_name}: {id}: location path `{}` not in the head or base tree",
                        l.path
                    ));
                }
            }
        };
        for e in &case.expected_findings {
            check_target(&e.id, &e.symbol, &e.location, &mut errors);
        }
        for f in &case.forbidden_findings {
            check_target(&f.id, &f.symbol, &f.location, &mut errors);
        }
        for o in &case.optional_findings {
            check_target(&o.id, &o.symbol, &o.location, &mut errors);
        }
        if case.expect_no_published_findings && !case.expected_findings.is_empty() {
            errors.push(format!(
                "{dir_name}: expect_no_published_findings requires no expected findings"
            ));
        }
        if case.has_class(CaseClass::SafeChange) != case.expect_no_published_findings {
            errors.push(format!(
                "{dir_name}: classes contain safe_change exactly when expect_no_published_findings is true"
            ));
        }
        let context = match fs::read_to_string(dir.join("context.json")) {
            Ok(text) => match serde_json::from_str::<ContextSnapshot>(&text) {
                Ok(c) => Some(c),
                Err(e) => {
                    errors.push(format!("{dir_name}: context.json: {e}"));
                    None
                }
            },
            Err(_) => None,
        };
        if !errors.is_empty() {
            return Err(errors);
        }
        let content_hash = dir_hash(dir).map_err(|e| vec![format!("{dir_name}: {e}")])?;
        Ok(LoadedCase {
            case,
            dir: dir.to_path_buf(),
            context,
            content_hash,
        })
    }
}

/// Case directories under `cases/`, sorted, optionally filtered by a glob on the case id.
pub fn case_dirs(paths: &EvalPaths, filter: Option<&str>) -> anyhow::Result<Vec<PathBuf>> {
    let matcher = match filter {
        Some(g) => Some(globset::Glob::new(g)?.compile_matcher()),
        None => None,
    };
    let mut dirs = Vec::new();
    for entry in fs::read_dir(paths.cases())? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if matcher.as_ref().is_some_and(|m| !m.is_match(&name)) {
            continue;
        }
        dirs.push(path);
    }
    dirs.sort();
    Ok(dirs)
}

/// Loads every selected case. Returns the valid cases and every error.
pub fn load_corpus(
    paths: &EvalPaths,
    filter: Option<&str>,
) -> anyhow::Result<(Vec<LoadedCase>, BTreeMap<String, Vec<String>>)> {
    let validator = CaseValidator::new(paths)?;
    let mut ok = Vec::new();
    let mut errors = BTreeMap::new();
    for dir in case_dirs(paths, filter)? {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        match validator.load(&dir) {
            Ok(c) => ok.push(c),
            Err(e) => {
                errors.insert(name, e);
            }
        }
    }
    Ok((ok, errors))
}
