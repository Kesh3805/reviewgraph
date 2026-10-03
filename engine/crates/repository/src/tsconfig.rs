//! tsconfig discovery, `extends` resolution and file ownership (INIT-007).
//!
//! tsconfig files are JSONC with `extends` chains. Merge rules follow TypeScript: compiler
//! options override per key, `paths` is replaced wholesale, relative paths stay relative to the
//! config that defined them, `include`/`exclude`/`files` are replaced and `references` are not
//! inherited. Configs are parsed, never executed.

use std::collections::{BTreeMap, BTreeSet};

use globset::{Glob, GlobMatcher};
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::manifests::ParseStatusLite;
use crate::read::BoundedReader;
use crate::walk::FileInventory;

const MAX_EXTENDS_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TsExtends {
    Local {
        path: RepoPath,
    },
    Package {
        specifier: String,
        resolved: Option<RepoPath>,
    },
    Unresolved {
        specifier: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TsPathMapping {
    pub pattern: String,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct TsEffectiveOptions {
    pub base_url: Option<RepoDir>,
    /// In declaration order: the order is significant to TypeScript.
    pub paths: Vec<TsPathMapping>,
    /// Directory of the config that defined `paths`; targets are relative to it unless a
    /// `baseUrl` is set.
    pub paths_base: RepoDir,
    pub module: Option<String>,
    pub module_resolution: Option<String>,
    pub target: Option<String>,
    pub jsx: Option<String>,
    pub strict: Option<bool>,
    pub experimental_decorators: Option<bool>,
    pub emit_decorator_metadata: Option<bool>,
    pub allow_js: Option<bool>,
    pub root_dir: Option<RepoDir>,
    pub out_dir: Option<RepoDir>,
    pub root_dirs: Vec<RepoDir>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Directory the include/exclude/files patterns are relative to.
    pub include_base: RepoDir,
    pub files: Vec<RepoPath>,
    pub references: Vec<RepoPath>,
    pub config_dir: RepoDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TsConfigFact {
    pub path: RepoPath,
    /// Nearest first.
    pub extends_chain: Vec<TsExtends>,
    pub effective: TsEffectiveOptions,
    pub parse_status: ParseStatusLite,
}

/// Result of [`TsConfigSet::tsconfig_for`].
#[derive(Debug, Clone, Copy)]
pub struct TsOwner<'a> {
    pub config: &'a TsConfigFact,
    /// No config's include/exclude matched; the nearest tsconfig.json was used.
    pub owned_by_fallback: bool,
}

/// Immutable after construction and `Sync`, so it can be shared through an `Arc`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct TsConfigSet {
    pub configs: Vec<TsConfigFact>,
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_tsconfig_name(name: &str) -> bool {
    (name.starts_with("tsconfig") && name.ends_with(".json")) || name == "jsconfig.json"
}

/// Names that ownership may choose.
fn is_primary_name(name: &str) -> bool {
    name == "tsconfig.json" || name == "jsconfig.json"
}

/// `dir/rel` normalized; `None` when it escapes the repository root.
pub fn join_normalized(dir: &RepoDir, rel: &str) -> Option<RepoDir> {
    let mut parts: Vec<&str> = if dir.is_root() {
        Vec::new()
    } else {
        dir.as_str().split('/').collect()
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    RepoDir::new(parts.join("/")).ok()
}

struct Loader<'a> {
    inv: &'a FileInventory,
    reader: &'a BoundedReader,
    raw: BTreeMap<String, Option<Value>>,
    warnings: Vec<InitWarning>,
}

impl Loader<'_> {
    fn load(&mut self, path: &str) -> Option<&Value> {
        if !self.raw.contains_key(path) {
            let value = self
                .inv
                .find(path)
                .and_then(|entry| self.reader.read_prefix(entry, 1024 * 1024).ok())
                .and_then(|bytes| crate::jsonc::parse_jsonc(&bytes).ok());
            self.raw.insert(path.to_owned(), value);
        }
        self.raw.get(path).and_then(|v| v.as_ref())
    }

    fn warn(&mut self, code: &str, path: &str, message: String) {
        self.warnings
            .push(InitWarning::new(code, RepoPath::new(path).ok(), message));
    }

    /// Resolves an `extends` specifier of the config at `from` (which lives in `dir`).
    fn resolve_extends(&mut self, from: &str, dir: &RepoDir, spec: &str) -> TsExtends {
        if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
            let mut candidates = vec![spec.to_owned()];
            if !spec.ends_with(".json") {
                candidates.push(format!("{spec}.json"));
                candidates.push(format!("{spec}/tsconfig.json"));
            }
            for candidate in candidates {
                if let Some(joined) = join_normalized(dir, &candidate) {
                    if let Ok(path) = RepoPath::new(joined.as_str()) {
                        if self.inv.find(path.as_str()).is_some() {
                            return TsExtends::Local { path };
                        }
                    }
                }
            }
            self.warn(
                "tsconfig_base_unresolved",
                from,
                format!("extends `{spec}` was not found in the repository"),
            );
            return TsExtends::Unresolved {
                specifier: spec.to_owned(),
            };
        }
        // Bare specifier: node_modules is never walked, so this normally stays unresolved.
        let mut candidates = vec![format!("node_modules/{spec}")];
        if !spec.ends_with(".json") {
            candidates.push(format!("node_modules/{spec}.json"));
            candidates.push(format!("node_modules/{spec}/tsconfig.json"));
        }
        for candidate in candidates {
            if self.inv.find(&candidate).is_some() {
                return TsExtends::Package {
                    specifier: spec.to_owned(),
                    resolved: RepoPath::new(candidate).ok(),
                };
            }
        }
        self.warn(
            "tsconfig_base_unresolved",
            from,
            format!("extends package `{spec}` is not available in the walked tree"),
        );
        TsExtends::Package {
            specifier: spec.to_owned(),
            resolved: None,
        }
    }
}

fn str_opt(options: &Value, key: &str) -> Option<String> {
    options.get(key).and_then(|v| v.as_str()).map(str::to_owned)
}

fn bool_opt(options: &Value, key: &str) -> Option<bool> {
    options.get(key).and_then(|v| v.as_bool())
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Effective options of the config at `path`, plus the extends chain (nearest first).
fn effective(
    loader: &mut Loader<'_>,
    path: &str,
    visited: &mut Vec<String>,
) -> Option<(TsEffectiveOptions, Vec<TsExtends>)> {
    if visited.len() >= MAX_EXTENDS_DEPTH {
        loader.warn(
            "tsconfig_extends_cycle",
            path,
            "extends chain is longer than 16 configs".to_owned(),
        );
        return None;
    }
    if visited.iter().any(|v| v == path) {
        loader.warn(
            "tsconfig_extends_cycle",
            path,
            "extends chain loops back to a config that is already being resolved".to_owned(),
        );
        return None;
    }
    let value = loader.load(path)?.clone();
    visited.push(path.to_owned());

    let config_dir = RepoPath::new(path)
        .map(|p| RepoDir::parent_of(&p))
        .unwrap_or_default();
    let mut result = TsEffectiveOptions {
        config_dir: config_dir.clone(),
        paths_base: config_dir.clone(),
        include_base: config_dir.clone(),
        ..TsEffectiveOptions::default()
    };
    let mut chain = Vec::new();

    // Bases, applied left to right.
    let specs: Vec<String> = match value.get("extends") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    let mut inherited = TsEffectiveOptions::default();
    let mut have_inherited = false;
    for spec in specs {
        let resolved = loader.resolve_extends(path, &config_dir, &spec);
        let base_path = match &resolved {
            TsExtends::Local { path } => Some(path.as_str().to_owned()),
            TsExtends::Package {
                resolved: Some(p), ..
            } => Some(p.as_str().to_owned()),
            _ => None,
        };
        chain.push(resolved);
        if let Some(base_path) = base_path {
            if let Some((base, base_chain)) = effective(loader, &base_path, visited) {
                merge(&mut inherited, &base);
                have_inherited = true;
                chain.extend(base_chain);
            }
        }
    }
    if have_inherited {
        // Everything inherited, except per-config identity and references.
        result = inherited;
        result.config_dir = config_dir.clone();
        result.references = Vec::new();
    }

    let own_options = value.get("compilerOptions").cloned().unwrap_or(Value::Null);
    let opts = &own_options;
    let resolve_dir = |rel: &str| join_normalized(&config_dir, rel);
    if let Some(v) = str_opt(opts, "baseUrl") {
        result.base_url = resolve_dir(&v);
    }
    if let Some(Value::Object(map)) = opts.get("paths") {
        result.paths = map
            .iter()
            .map(|(pattern, targets)| TsPathMapping {
                pattern: pattern.clone(),
                targets: string_list(Some(targets)),
            })
            .collect();
        result.paths_base = config_dir.clone();
    }
    if let Some(v) = str_opt(opts, "module") {
        result.module = Some(v);
    }
    if let Some(v) = str_opt(opts, "moduleResolution") {
        result.module_resolution = Some(v);
    }
    if let Some(v) = str_opt(opts, "target") {
        result.target = Some(v);
    }
    if let Some(v) = str_opt(opts, "jsx") {
        result.jsx = Some(v);
    }
    if let Some(v) = bool_opt(opts, "strict") {
        result.strict = Some(v);
    }
    if let Some(v) = bool_opt(opts, "experimentalDecorators") {
        result.experimental_decorators = Some(v);
    }
    if let Some(v) = bool_opt(opts, "emitDecoratorMetadata") {
        result.emit_decorator_metadata = Some(v);
    }
    if let Some(v) = bool_opt(opts, "allowJs") {
        result.allow_js = Some(v);
    }
    if let Some(v) = str_opt(opts, "rootDir") {
        result.root_dir = resolve_dir(&v);
    }
    if let Some(v) = str_opt(opts, "outDir") {
        result.out_dir = resolve_dir(&v);
    }
    if opts.get("rootDirs").is_some() {
        result.root_dirs = string_list(opts.get("rootDirs"))
            .iter()
            .filter_map(|d| resolve_dir(d))
            .collect();
    }
    let own_include = value.get("include").is_some();
    let own_exclude = value.get("exclude").is_some();
    let own_files = value.get("files").is_some();
    if own_include || own_files || own_exclude {
        result.include_base = config_dir.clone();
    }
    if own_include {
        result.include = string_list(value.get("include"));
    }
    if own_exclude {
        result.exclude = string_list(value.get("exclude"));
    }
    if own_files {
        result.files = string_list(value.get("files"))
            .iter()
            .filter_map(|f| {
                join_normalized(&config_dir, f).and_then(|d| RepoPath::new(d.as_str()).ok())
            })
            .collect();
    }
    result.references = value
        .get("references")
        .and_then(|v| v.as_array())
        .map(|refs| {
            refs.iter()
                .filter_map(|r| r.get("path").and_then(|p| p.as_str()))
                .filter_map(|p| join_normalized(&config_dir, p))
                .filter_map(|d| {
                    let as_file = if d.as_str().ends_with(".json") {
                        d.as_str().to_owned()
                    } else {
                        format!("{d}/tsconfig.json")
                    };
                    RepoPath::new(as_file).ok()
                })
                .collect()
        })
        .unwrap_or_default();

    visited.pop();
    Some((result, chain))
}

/// Copies every set field of `base` over `into` (left-to-right merge of `extends` arrays).
fn merge(into: &mut TsEffectiveOptions, base: &TsEffectiveOptions) {
    macro_rules! over {
        ($($f:ident),*) => { $( if base.$f.is_some() { into.$f = base.$f.clone(); } )* };
    }
    over!(
        base_url,
        module,
        module_resolution,
        target,
        jsx,
        strict,
        experimental_decorators,
        emit_decorator_metadata,
        allow_js,
        root_dir,
        out_dir
    );
    if !base.paths.is_empty() {
        into.paths = base.paths.clone();
        into.paths_base = base.paths_base.clone();
    }
    if !base.root_dirs.is_empty() {
        into.root_dirs = base.root_dirs.clone();
    }
    if !base.include.is_empty() || !base.exclude.is_empty() || !base.files.is_empty() {
        into.include = base.include.clone();
        into.exclude = base.exclude.clone();
        into.files = base.files.clone();
        into.include_base = base.include_base.clone();
    }
}

/// Discovers and resolves every tsconfig/jsconfig in the inventory.
pub fn detect_tsconfigs(
    inv: &FileInventory,
    reader: &BoundedReader,
) -> (TsConfigSet, Vec<InitWarning>) {
    let mut loader = Loader {
        inv,
        reader,
        raw: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let mut configs = Vec::new();
    let paths: Vec<RepoPath> = inv
        .entries
        .iter()
        .filter(|e| is_tsconfig_name(basename(e.path.as_str())))
        .map(|e| e.path.clone())
        .collect();
    for path in paths {
        let parsed = loader.load(path.as_str()).is_some();
        if !parsed {
            loader.warn(
                "tsconfig_parse",
                path.as_str(),
                "config is not valid JSON".to_owned(),
            );
            let dir = RepoDir::parent_of(&path);
            configs.push(TsConfigFact {
                path: path.clone(),
                extends_chain: Vec::new(),
                effective: TsEffectiveOptions {
                    config_dir: dir.clone(),
                    paths_base: dir.clone(),
                    include_base: dir,
                    ..TsEffectiveOptions::default()
                },
                parse_status: ParseStatusLite::Failed {
                    reason: "invalid JSON".to_owned(),
                },
            });
            continue;
        }
        let mut visited = Vec::new();
        let Some((mut effective, chain)) = effective(&mut loader, path.as_str(), &mut visited)
        else {
            continue;
        };
        if effective.include.is_empty() && effective.files.is_empty() {
            effective.include = vec!["**/*".to_owned()];
        }
        configs.push(TsConfigFact {
            path,
            extends_chain: chain,
            effective,
            parse_status: ParseStatusLite::Ok,
        });
    }
    configs.sort_by(|a, b| a.path.cmp(&b.path));
    let mut warnings = loader.warnings;
    warnings.sort();
    warnings.dedup();
    (TsConfigSet { configs }, warnings)
}

fn matcher(pattern: &str) -> Option<GlobMatcher> {
    let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
    Glob::new(pattern).ok().map(|g| g.compile_matcher())
}

/// TypeScript pattern semantics: a pattern matches a path or any of its parent directories.
fn pattern_matches(matcher: &GlobMatcher, rel: &str) -> bool {
    if matcher.is_match(rel) {
        return true;
    }
    let mut end = rel.len();
    while let Some(idx) = rel[..end].rfind('/') {
        end = idx;
        if matcher.is_match(&rel[..end]) {
            return true;
        }
    }
    false
}

fn has_supported_extension(path: &str, allow_js: bool) -> bool {
    let ts = [".ts", ".tsx", ".mts", ".cts"];
    let js = [".js", ".jsx", ".mjs", ".cjs"];
    ts.iter().any(|e| path.ends_with(e)) || (allow_js && js.iter().any(|e| path.ends_with(e)))
}

impl TsConfigSet {
    pub fn get(&self, path: &str) -> Option<&TsConfigFact> {
        self.configs.iter().find(|c| c.path.as_str() == path)
    }

    fn includes(&self, config: &TsConfigFact, file: &RepoPath) -> bool {
        let e = &config.effective;
        let allow_js =
            e.allow_js.unwrap_or(false) || basename(config.path.as_str()) == "jsconfig.json";
        if !e.include_base.contains(file.as_str()) {
            return false;
        }
        if e.files.iter().any(|f| f == file) {
            return true;
        }
        if !has_supported_extension(file.as_str(), allow_js) {
            return false;
        }
        let rel = if e.include_base.is_root() {
            file.as_str()
        } else {
            file.as_str()[e.include_base.as_str().len()..].trim_start_matches('/')
        };
        if e.exclude
            .iter()
            .filter_map(|p| matcher(p))
            .any(|m| pattern_matches(&m, rel))
        {
            return false;
        }
        e.include
            .iter()
            .filter_map(|p| matcher(p))
            .any(|m| pattern_matches(&m, rel))
    }

    /// The tsconfig that governs `file`: the nearest `tsconfig.json`/`jsconfig.json` in an
    /// ancestor directory whose include/exclude match; otherwise the nearest one, flagged.
    /// `tsconfig.build.json`, `tsconfig.spec.json` and similar are never chosen.
    pub fn tsconfig_for(&self, file: &RepoPath) -> Option<TsOwner<'_>> {
        let mut dir = RepoDir::parent_of(file);
        let mut nearest: Option<&TsConfigFact> = None;
        loop {
            for candidate in self.configs.iter().filter(|c| {
                is_primary_name(basename(c.path.as_str()))
                    && c.effective.config_dir == dir
                    && !matches!(c.parse_status, ParseStatusLite::Failed { .. })
            }) {
                if nearest.is_none() {
                    nearest = Some(candidate);
                }
                if self.includes(candidate, file) {
                    return Some(TsOwner {
                        config: candidate,
                        owned_by_fallback: false,
                    });
                }
            }
            if dir.is_root() {
                break;
            }
            dir = match dir.as_str().rsplit_once('/') {
                Some((parent, _)) => RepoDir::new(parent).ok()?,
                None => RepoDir::root(),
            };
        }
        nearest.map(|config| TsOwner {
            config,
            owned_by_fallback: true,
        })
    }

    /// Output directories of all configs (used by generated-code detection).
    pub fn out_dirs(&self) -> BTreeSet<RepoDir> {
        self.configs
            .iter()
            .filter_map(|c| c.effective.out_dir.clone())
            .filter(|d| !d.is_root())
            .collect()
    }
}
