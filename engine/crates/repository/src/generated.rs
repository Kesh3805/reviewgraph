//! Generated, vendored, lockfile and minified file detection (INIT-008).
//!
//! Every verdict carries the single reason that won, chosen by a fixed precedence:
//! config exclude, config include, `.gitattributes`, header marker, file-name pattern,
//! directory name. The losing reasons are discarded, so the result is explainable and stable.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use globset::{Glob, GlobMatcher};
use rayon::prelude::*;
use regex::Regex;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::read::BoundedReader;
use crate::tsconfig::TsConfigSet;
use crate::walk::{FileClass, FileEntry, FileInventory};

const HEADER_BYTES: usize = 2048;
const HEADER_LINES: usize = 20;
const GITATTRIBUTES_MAX: usize = 64 * 1024;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedKind {
    Generated,
    Vendored,
    Lockfile,
    Minified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum GeneratedReason {
    ConfigInclude { glob: String },
    ConfigExclude { glob: String },
    GitAttributes { attribute: String },
    Header { marker: String, line: u32 },
    Pattern { pattern: String },
    Directory { name: String },
    TsOutDir { dir: RepoPath },
}

impl GeneratedReason {
    /// Metric / summary label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ConfigInclude { .. } => "config_include",
            Self::ConfigExclude { .. } => "config_exclude",
            Self::GitAttributes { .. } => "git_attributes",
            Self::Header { .. } => "header",
            Self::Pattern { .. } => "pattern",
            Self::Directory { .. } => "directory",
            Self::TsOutDir { .. } => "ts_out_dir",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GeneratedClass {
    pub kind: GeneratedKind,
    pub reason: GeneratedReason,
    pub confidence: f32,
    pub generator_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct GeneratedFacts {
    pub files: BTreeMap<RepoPath, GeneratedClass>,
    /// Directories that matched by name or `outDir`, with the number of files under them.
    pub dirs: Vec<(RepoPath, GeneratedKind, u64)>,
    pub config_globs: Vec<String>,
}

impl GeneratedFacts {
    pub fn is_generated(&self, path: &RepoPath) -> bool {
        self.files
            .get(path)
            .is_some_and(|c| c.kind == GeneratedKind::Generated)
    }
}

/// `.review/config.yaml` `generated:` section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneratedConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttrValue {
    Set,
    Unset,
}

struct AttrRule {
    base: RepoDir,
    matcher: GlobMatcher,
    generated: Option<AttrValue>,
    vendored: Option<AttrValue>,
}

/// Parsed `.gitattributes` files (root and nested).
#[derive(Default)]
pub struct GitAttributesView {
    rules: Vec<AttrRule>,
}

impl std::fmt::Debug for GitAttributesView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GitAttributesView({} rules)", self.rules.len())
    }
}

fn attr_value(token: &str, name: &str) -> Option<AttrValue> {
    if token == name || token == format!("{name}=true") {
        Some(AttrValue::Set)
    } else if token == format!("-{name}")
        || token == format!("!{name}")
        || token == format!("{name}=false")
    {
        Some(AttrValue::Unset)
    } else {
        None
    }
}

impl GitAttributesView {
    pub fn from_inventory(inv: &FileInventory, reader: &BoundedReader) -> Self {
        let mut rules = Vec::new();
        for entry in inv
            .entries
            .iter()
            .filter(|e| e.path.as_str().rsplit('/').next() == Some(".gitattributes"))
        {
            let Ok(text) = reader.read_text(entry, GITATTRIBUTES_MAX) else {
                continue;
            };
            let base = RepoDir::parent_of(&entry.path);
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let Some(pattern) = parts.next() else {
                    continue;
                };
                let (mut generated, mut vendored) = (None, None);
                for token in parts {
                    if let Some(v) = attr_value(token, "linguist-generated") {
                        generated = Some(v);
                    }
                    if let Some(v) = attr_value(token, "linguist-vendored") {
                        vendored = Some(v);
                    }
                }
                if generated.is_none() && vendored.is_none() {
                    continue;
                }
                // Patterns without a slash match at any depth below the file's directory.
                let anchored = pattern.trim_start_matches('/');
                let glob = if pattern.contains('/') {
                    anchored.trim_end_matches('/').to_owned()
                } else {
                    format!("**/{anchored}")
                };
                let glob = if pattern.ends_with('/') {
                    format!("{glob}/**")
                } else {
                    glob
                };
                if let Ok(g) = Glob::new(&glob) {
                    rules.push(AttrRule {
                        base: base.clone(),
                        matcher: g.compile_matcher(),
                        generated,
                        vendored,
                    });
                }
            }
        }
        // Deeper files override shallower ones; within a file, later lines override.
        rules.sort_by_key(|r: &AttrRule| r.base.depth());
        Self { rules }
    }

    fn lookup(&self, path: &str) -> (Option<AttrValue>, Option<AttrValue>) {
        let (mut generated, mut vendored) = (None, None);
        for rule in &self.rules {
            if !rule.base.contains(path) {
                continue;
            }
            let rel = if rule.base.is_root() {
                path
            } else {
                path[rule.base.as_str().len()..].trim_start_matches('/')
            };
            if rule.matcher.is_match(rel) {
                if rule.generated.is_some() {
                    generated = rule.generated;
                }
                if rule.vendored.is_some() {
                    vendored = rule.vendored;
                }
            }
        }
        (generated, vendored)
    }
}

const GENERATED_DIRS: &[&str] = &[
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "coverage",
    "generated",
    "__generated__",
    "gen",
    ".turbo",
    ".nx",
    ".serverless",
    "storybook-static",
    "allure-report",
    "allure-results",
];
const VENDORED_DIRS: &[&str] = &["vendor", "third_party", "third-party"];

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "Cargo.lock",
    "go.sum",
    "poetry.lock",
    "uv.lock",
    "composer.lock",
    "Gemfile.lock",
];

struct PatternTable {
    basename: Vec<(GlobMatcher, &'static str, GeneratedKind, f32)>,
    path: Vec<(GlobMatcher, &'static str, GeneratedKind, f32)>,
    go_marker: Option<Regex>,
    hint: Option<Regex>,
}

fn table() -> &'static PatternTable {
    static TABLE: OnceLock<PatternTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mk = |p: &'static str, k, c| Glob::new(p).ok().map(|g| (g.compile_matcher(), p, k, c));
        let basename: Vec<_> = [
            ("*.generated.*", GeneratedKind::Generated, 0.9),
            ("*.gen.ts", GeneratedKind::Generated, 0.9),
            ("*.gen.js", GeneratedKind::Generated, 0.9),
            ("*.gen.go", GeneratedKind::Generated, 0.9),
            ("*.pb.go", GeneratedKind::Generated, 0.95),
            ("*_pb2.py", GeneratedKind::Generated, 0.95),
            ("*_pb2_grpc.py", GeneratedKind::Generated, 0.95),
            ("*.pb.ts", GeneratedKind::Generated, 0.95),
            ("*.pb.js", GeneratedKind::Generated, 0.95),
            ("*_grpc_pb.ts", GeneratedKind::Generated, 0.95),
            ("*_grpc_pb.js", GeneratedKind::Generated, 0.95),
            ("*.min.js", GeneratedKind::Minified, 0.9),
            ("*.min.css", GeneratedKind::Minified, 0.9),
        ]
        .into_iter()
        .filter_map(|(p, k, c)| mk(p, k, c))
        .collect();
        let path: Vec<_> = [
            ("**/migrations/*.snap", GeneratedKind::Generated, 0.7),
            ("**/__snapshots__/*.snap", GeneratedKind::Generated, 0.7),
        ]
        .into_iter()
        .filter_map(|(p, k, c)| mk(p, k, c))
        .collect();
        PatternTable {
            basename,
            path,
            go_marker: Regex::new(r"^//\s*Code generated .* DO NOT EDIT\.?\s*$").ok(),
            hint: Regex::new(r"Code generated by (\S+)").ok(),
        }
    })
}

fn is_comment_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//")
        || t.starts_with('#')
        || t.starts_with("/*")
        || t.starts_with('*')
        || t.starts_with("<!--")
        || t.starts_with("--")
}

fn header_scan(text: &str) -> Option<(String, u32, Option<String>, f32)> {
    let table = table();
    for (i, line) in text.lines().take(HEADER_LINES).enumerate() {
        if !is_comment_line(line) {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let line_no = (i + 1) as u32;
        let hint = table
            .hint
            .as_ref()
            .and_then(|re| re.captures(line))
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim_end_matches(['.', ',']).to_owned());
        if line.contains("@generated") {
            return Some(("@generated".to_owned(), line_no, hint, 0.95));
        }
        if table
            .go_marker
            .as_ref()
            .is_some_and(|re| re.is_match(line.trim()))
        {
            return Some(("code-generated-do-not-edit".to_owned(), line_no, hint, 0.95));
        }
        if lower.contains("this file is automatically generated") {
            return Some(("automatically-generated".to_owned(), line_no, hint, 0.95));
        }
        if lower.contains("auto-generated file") {
            return Some(("auto-generated-file".to_owned(), line_no, hint, 0.95));
        }
        if lower.contains("generat") {
            for loose in ["do not edit", "auto-generated", "autogenerated"] {
                if lower.contains(loose) {
                    return Some((loose.replace(' ', "-"), line_no, hint, 0.7));
                }
            }
        }
    }
    None
}

/// Directory-name verdict: `(kind, matched dir path, segment, confidence)`.
fn directory_verdict(
    path: &str,
    out_dirs: &[RepoDir],
) -> Option<(GeneratedKind, RepoPath, GeneratedReason, f32)> {
    for dir in out_dirs {
        if dir.contains(path) && path != dir.as_str() {
            let rp = RepoPath::new(dir.as_str()).ok()?;
            return Some((
                GeneratedKind::Generated,
                rp.clone(),
                GeneratedReason::TsOutDir { dir: rp },
                0.9,
            ));
        }
    }
    let segments: Vec<&str> = path.split('/').collect();
    let mut prefix_len = 0;
    for (i, segment) in segments
        .iter()
        .enumerate()
        .take(segments.len().saturating_sub(1))
    {
        prefix_len += segment.len() + usize::from(i > 0);
        let kind = if GENERATED_DIRS.contains(segment) {
            GeneratedKind::Generated
        } else if VENDORED_DIRS.contains(segment) {
            GeneratedKind::Vendored
        } else {
            continue;
        };
        let dir = RepoPath::new(&path[..prefix_len]).ok()?;
        return Some((
            kind,
            dir,
            GeneratedReason::Directory {
                name: (*segment).to_owned(),
            },
            0.8,
        ));
    }
    None
}

enum Verdict {
    NotGenerated,
    Class(GeneratedClass, Option<RepoPath>),
    None,
}

struct Compiled {
    include: Vec<(GlobMatcher, String)>,
    exclude: Vec<(GlobMatcher, String)>,
}

fn compile_globs(globs: &[String], warnings: &mut Vec<InitWarning>) -> Vec<(GlobMatcher, String)> {
    globs
        .iter()
        .filter_map(|g| match Glob::new(g.trim_start_matches("./")) {
            Ok(glob) => Some((glob.compile_matcher(), g.clone())),
            Err(_) => {
                warnings.push(InitWarning::new(
                    "invalid_generated_glob",
                    None,
                    format!("generated glob `{g}` is invalid and was skipped"),
                ));
                None
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn classify_one(
    entry: &FileEntry,
    compiled: &Compiled,
    attrs: &GitAttributesView,
    out_dirs: &[RepoDir],
    reader: &BoundedReader,
    warnings: &mut Vec<InitWarning>,
) -> Verdict {
    let path = entry.path.as_str();
    let name = path.rsplit('/').next().unwrap_or(path);

    // 1. config exclude: force not generated.
    if compiled.exclude.iter().any(|(m, _)| m.is_match(path)) {
        return Verdict::NotGenerated;
    }
    // 2. config include.
    if let Some((_, glob)) = compiled.include.iter().find(|(m, _)| m.is_match(path)) {
        return Verdict::Class(
            GeneratedClass {
                kind: GeneratedKind::Generated,
                reason: GeneratedReason::ConfigInclude { glob: glob.clone() },
                confidence: 1.0,
                generator_hint: None,
            },
            None,
        );
    }
    // 3. .gitattributes.
    let (generated, vendored) = attrs.lookup(path);
    let any_unset = generated == Some(AttrValue::Unset) || vendored == Some(AttrValue::Unset);
    let any_set = generated == Some(AttrValue::Set) || vendored == Some(AttrValue::Set);
    if any_unset && !any_set {
        // An explicit "not generated"/"not vendored" wins over every inferred reason.
        return Verdict::NotGenerated;
    }
    if generated == Some(AttrValue::Set) {
        return Verdict::Class(
            GeneratedClass {
                kind: GeneratedKind::Generated,
                reason: GeneratedReason::GitAttributes {
                    attribute: "linguist-generated".to_owned(),
                },
                confidence: 1.0,
                generator_hint: None,
            },
            None,
        );
    }
    if vendored == Some(AttrValue::Set) {
        return Verdict::Class(
            GeneratedClass {
                kind: GeneratedKind::Vendored,
                reason: GeneratedReason::GitAttributes {
                    attribute: "linguist-vendored".to_owned(),
                },
                confidence: 1.0,
                generator_hint: None,
            },
            None,
        );
    }
    // 4. header marker (Source files only).
    if entry.class == FileClass::Source {
        match reader.read_text(entry, HEADER_BYTES) {
            Ok(text) => {
                if let Some((marker, line, hint, confidence)) = header_scan(&text) {
                    return Verdict::Class(
                        GeneratedClass {
                            kind: GeneratedKind::Generated,
                            reason: GeneratedReason::Header { marker, line },
                            confidence,
                            generator_hint: hint,
                        },
                        None,
                    );
                }
            }
            Err(_) => warnings.push(InitWarning::new(
                "unreadable_entry",
                Some(entry.path.clone()),
                "file header could not be read",
            )),
        }
    }
    // 5. file-name patterns.
    if LOCKFILES.contains(&name) {
        return Verdict::Class(
            GeneratedClass {
                kind: GeneratedKind::Lockfile,
                reason: GeneratedReason::Pattern {
                    pattern: name.to_owned(),
                },
                confidence: 1.0,
                generator_hint: None,
            },
            None,
        );
    }
    let table = table();
    for (m, pattern, kind, confidence) in &table.basename {
        if m.is_match(name) {
            return Verdict::Class(
                GeneratedClass {
                    kind: *kind,
                    reason: GeneratedReason::Pattern {
                        pattern: (*pattern).to_owned(),
                    },
                    confidence: *confidence,
                    generator_hint: None,
                },
                None,
            );
        }
    }
    for (m, pattern, kind, confidence) in &table.path {
        if m.is_match(path) {
            return Verdict::Class(
                GeneratedClass {
                    kind: *kind,
                    reason: GeneratedReason::Pattern {
                        pattern: (*pattern).to_owned(),
                    },
                    confidence: *confidence,
                    generator_hint: None,
                },
                None,
            );
        }
    }
    // 6. directory names and tsconfig outDir.
    if let Some((kind, dir, reason, confidence)) = directory_verdict(path, out_dirs) {
        return Verdict::Class(
            GeneratedClass {
                kind,
                reason,
                confidence,
                generator_hint: None,
            },
            Some(dir),
        );
    }
    Verdict::None
}

/// Classifies generated, vendored, lockfile and minified files.
pub fn classify_generated(
    inv: &FileInventory,
    ts: &TsConfigSet,
    attrs: &GitAttributesView,
    cfg: &GeneratedConfig,
    reader: &BoundedReader,
) -> (GeneratedFacts, Vec<InitWarning>) {
    let span = tracing::info_span!(
        "init.generated",
        generated.files = tracing::field::Empty,
        generated.dirs = tracing::field::Empty
    );
    let _guard = span.enter();

    let mut warnings = Vec::new();
    let compiled = Compiled {
        include: compile_globs(&cfg.include, &mut warnings),
        exclude: compile_globs(&cfg.exclude, &mut warnings),
    };
    let out_dirs: Vec<RepoDir> = ts.out_dirs().into_iter().collect();

    let results: Vec<(RepoPath, Verdict, Vec<InitWarning>)> = inv
        .entries
        .par_iter()
        .filter(|e| !matches!(e.class, FileClass::Symlink))
        .map(|entry| {
            let mut w = Vec::new();
            let verdict = classify_one(entry, &compiled, attrs, &out_dirs, reader, &mut w);
            (entry.path.clone(), verdict, w)
        })
        .collect();

    let mut facts = GeneratedFacts {
        config_globs: cfg.include.clone(),
        ..GeneratedFacts::default()
    };
    let mut dir_counts: BTreeMap<(RepoPath, GeneratedKind), u64> = BTreeMap::new();
    for (path, verdict, w) in results {
        warnings.extend(w);
        if let Verdict::Class(class, dir) = verdict {
            if let Some(dir) = dir {
                *dir_counts.entry((dir, class.kind)).or_insert(0) += 1;
            }
            facts.files.insert(path, class);
        }
    }
    facts.dirs = dir_counts
        .into_iter()
        .map(|((dir, kind), n)| (dir, kind, n))
        .collect();
    warnings.sort();
    warnings.dedup();
    span.record("generated.files", facts.files.len());
    span.record("generated.dirs", facts.dirs.len());
    (facts, warnings)
}
