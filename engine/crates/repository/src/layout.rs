//! Source roots, test roots and test globs (INIT-007).
//!
//! Jest/Vitest configuration written as JavaScript or TypeScript is never executed: only the
//! string literals following `roots:`, `testMatch:` and `testRegex:` are extracted statically,
//! at lower confidence.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::read::BoundedReader;
use crate::tsconfig::{join_normalized, TsConfigSet};
use crate::walk::FileInventory;
use crate::workspaces::WorkspaceLayout;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RootSource {
    TsRootDir,
    NestCliSourceRoot,
    WorkspaceSrc,
    Conventional,
    ConventionalTest,
    JestConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RootFact {
    pub path: RepoDir,
    pub source: RootSource,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct LayoutFacts {
    pub source_roots: Vec<RootFact>,
    pub test_roots: Vec<RootFact>,
    pub test_globs: Vec<String>,
    /// `testRegex` values from Jest configuration, verbatim.
    pub test_regexes: Vec<String>,
}

const DEFAULT_TEST_GLOBS: &[&str] = &[
    "**/*.{spec,test}.{ts,tsx,js,jsx,mts,cts,mjs,cjs}",
    "**/*.e2e-spec.ts",
    "**/__tests__/**/*.{ts,tsx,js,jsx}",
];
const CONVENTIONAL_TEST_DIRS: &[&str] = &["test", "tests", "__tests__", "e2e", "spec"];
const CONVENTIONAL_SOURCE_DIRS: &[&str] = &["src", "lib", "app"];

fn jest_regexes() -> &'static Option<(Regex, Regex)> {
    static RE: OnceLock<Option<(Regex, Regex)>> = OnceLock::new();
    RE.get_or_init(|| {
        Some((
            Regex::new(
                r#"\b(roots|testMatch|testRegex)\s*:\s*(\[[^\]]*\]|"[^"]*"|'[^']*'|`[^`]*`)"#,
            )
            .ok()?,
            Regex::new(r#""([^"]*)"|'([^']*)'|`([^`]*)`"#).ok()?,
        ))
    })
}

#[derive(Default)]
struct JestFacts {
    roots: Vec<String>,
    test_match: Vec<String>,
    test_regex: Vec<String>,
    root_dir: Option<String>,
}

fn jest_from_json(value: &Value) -> JestFacts {
    let list = |key: &str| -> Vec<String> {
        match value.get(key) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        }
    };
    JestFacts {
        roots: list("roots"),
        test_match: list("testMatch"),
        test_regex: list("testRegex"),
        root_dir: value
            .get("rootDir")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
    }
}

fn jest_from_source(text: &str) -> JestFacts {
    let mut facts = JestFacts::default();
    let Some((outer, literal)) = jest_regexes() else {
        return facts;
    };
    for caps in outer.captures_iter(text) {
        let key = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let body = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let values: Vec<String> = literal
            .captures_iter(body)
            .filter_map(|c| {
                (1..=3)
                    .find_map(|i| c.get(i))
                    .map(|m| m.as_str().to_owned())
            })
            .collect();
        match key {
            "roots" => facts.roots.extend(values),
            "testMatch" => facts.test_match.extend(values),
            _ => facts.test_regex.extend(values),
        }
    }
    facts
}

/// Detects source roots, test roots and test globs.
pub fn detect_layout(
    inv: &FileInventory,
    reader: &BoundedReader,
    tsconfigs: &TsConfigSet,
    workspaces: &WorkspaceLayout,
) -> (LayoutFacts, Vec<InitWarning>) {
    let span = tracing::info_span!(
        "init.layout",
        tsconfig.count = tsconfigs.configs.len(),
        source_roots.count = tracing::field::Empty,
        test_roots.count = tracing::field::Empty
    );
    let _guard = span.enter();

    let mut warnings = Vec::new();
    let mut source: BTreeMap<RepoDir, RootFact> = BTreeMap::new();
    let mut tests: BTreeMap<RepoDir, RootFact> = BTreeMap::new();
    let put = |map: &mut BTreeMap<RepoDir, RootFact>, path: RepoDir, src: RootSource, conf: f32| {
        match map.get(&path) {
            Some(existing) if existing.confidence >= conf => {}
            _ => {
                map.insert(
                    path.clone(),
                    RootFact {
                        path,
                        source: src,
                        confidence: conf,
                    },
                );
            }
        }
    };
    let has_dir = |d: &RepoDir| inv.dirs.iter().any(|x| x.as_str() == d.as_str());
    let has_code_under = |d: &RepoDir| {
        inv.entries.iter().any(|e| {
            d.contains(e.path.as_str())
                && matches!(
                    e.path.extension(),
                    Some("ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs")
                )
        })
    };

    // tsconfig rootDir.
    for config in &tsconfigs.configs {
        let name = config.path.as_str().rsplit('/').next().unwrap_or("");
        if name != "tsconfig.json" && name != "jsconfig.json" {
            continue;
        }
        if let Some(root) = config.effective.root_dir.as_ref().filter(|r| !r.is_root()) {
            put(&mut source, root.clone(), RootSource::TsRootDir, 0.9);
        }
    }

    // nest-cli.json sourceRoot (+ projects.*.sourceRoot).
    for entry in inv
        .entries
        .iter()
        .filter(|e| e.path.as_str().rsplit('/').next() == Some("nest-cli.json"))
    {
        let dir = RepoDir::parent_of(&entry.path);
        let Some(value) = reader
            .read_text(entry, 256 * 1024)
            .ok()
            .and_then(|t| crate::jsonc::parse_jsonc(t.as_bytes()).ok())
        else {
            continue;
        };
        let mut roots: Vec<String> = Vec::new();
        if let Some(s) = value.get("sourceRoot").and_then(|v| v.as_str()) {
            roots.push(s.to_owned());
        }
        if let Some(projects) = value.get("projects").and_then(|v| v.as_object()) {
            roots.extend(
                projects
                    .values()
                    .filter_map(|p| p.get("sourceRoot").and_then(|v| v.as_str()))
                    .map(str::to_owned),
            );
        }
        for root in roots {
            if let Some(path) = join_normalized(&dir, &root).filter(|d| !d.is_root()) {
                put(&mut source, path, RootSource::NestCliSourceRoot, 0.95);
            }
        }
    }

    // Workspace package src/ dirs.
    for pkg in &workspaces.packages {
        if let Ok(path) = pkg.dir.join("src") {
            let dir = RepoDir::from(path);
            if has_dir(&dir) {
                put(&mut source, dir, RootSource::WorkspaceSrc, 0.8);
            }
        }
    }

    // Conventional roots at the repository root.
    for name in CONVENTIONAL_SOURCE_DIRS {
        if let Ok(dir) = RepoDir::new(*name) {
            if has_dir(&dir) && has_code_under(&dir) {
                put(&mut source, dir, RootSource::Conventional, 0.6);
            }
        }
    }

    // Conventional test directories (root and one level of package dirs).
    for dir in &inv.dirs {
        let name = dir.as_str().rsplit('/').next().unwrap_or("");
        if CONVENTIONAL_TEST_DIRS.contains(&name) && dir.as_str().matches('/').count() <= 2 {
            put(
                &mut tests,
                RepoDir::from(dir.clone()),
                RootSource::ConventionalTest,
                0.8,
            );
        }
    }

    // Jest configuration.
    let mut test_globs: Vec<String> = DEFAULT_TEST_GLOBS.iter().map(|g| (*g).to_owned()).collect();
    let mut test_regexes = Vec::new();
    let mut apply = |facts: JestFacts,
                     dir: &RepoDir,
                     confidence: f32,
                     tests: &mut BTreeMap<RepoDir, RootFact>,
                     test_globs: &mut Vec<String>| {
        let base = match facts.root_dir.as_deref() {
            Some(rd) => join_normalized(dir, rd).unwrap_or_else(|| dir.clone()),
            None => dir.clone(),
        };
        for root in &facts.roots {
            let rel = root.replace("<rootDir>", "");
            let rel = rel.trim_start_matches('/');
            if let Some(path) = join_normalized(&base, rel).filter(|d| !d.is_root()) {
                put(tests, path, RootSource::JestConfig, confidence);
            }
        }
        if facts.roots.is_empty() && !base.is_root() && facts.root_dir.is_some() {
            put(tests, base.clone(), RootSource::JestConfig, confidence);
        }
        for m in facts.test_match {
            let m = m.replace("<rootDir>/", "");
            if !test_globs.contains(&m) {
                test_globs.push(m);
            }
        }
        test_regexes.extend(facts.test_regex);
    };
    for entry in &inv.entries {
        let name = entry.path.as_str().rsplit('/').next().unwrap_or("");
        let dir = RepoDir::parent_of(&entry.path);
        let is_json_config =
            name == "jest.config.json" || (name.starts_with("jest") && name.ends_with(".json"));
        let is_script_config = matches!(
            name,
            "jest.config.ts" | "jest.config.js" | "jest.config.mjs" | "jest.config.cjs"
        );
        if name == "package.json" {
            if let Some(value) = reader
                .read_text(entry, 2 * 1024 * 1024)
                .ok()
                .and_then(|t| crate::jsonc::parse_jsonc(t.as_bytes()).ok())
            {
                if let Some(jest) = value.get("jest") {
                    apply(jest_from_json(jest), &dir, 0.9, &mut tests, &mut test_globs);
                }
            }
        } else if is_json_config {
            if let Some(value) = reader
                .read_text(entry, 256 * 1024)
                .ok()
                .and_then(|t| crate::jsonc::parse_jsonc(t.as_bytes()).ok())
            {
                apply(
                    jest_from_json(&value),
                    &dir,
                    0.9,
                    &mut tests,
                    &mut test_globs,
                );
            }
        } else if is_script_config {
            if let Ok(text) = reader.read_text(entry, 256 * 1024) {
                warnings.push(InitWarning::new(
                    "config_static_extraction",
                    Some(entry.path.clone()),
                    "script config was scanned statically and never executed",
                ));
                apply(
                    jest_from_source(&text),
                    &dir,
                    0.6,
                    &mut tests,
                    &mut test_globs,
                );
            }
        }
    }

    let mut source_roots: Vec<RootFact> = source.into_values().collect();
    let mut test_roots: Vec<RootFact> = tests.into_values().collect();
    for list in [&mut source_roots, &mut test_roots] {
        list.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
        });
    }
    test_regexes.sort();
    test_regexes.dedup();
    span.record("source_roots.count", source_roots.len());
    span.record("test_roots.count", test_roots.len());
    warnings.sort();
    (
        LayoutFacts {
            source_roots,
            test_roots,
            test_globs,
            test_regexes,
        },
        warnings,
    )
}
