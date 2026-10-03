//! Package-manager and manifest detection (INIT-004).
//!
//! Manifests are parsed leniently: a wrong type on one field makes the manifest `Partial`, a
//! syntax error makes it `Failed`, and neither is fatal. Script commands are never stored.

mod cargo;
mod go;
mod jvm;
mod npm;
mod other;
mod python;

use std::collections::{BTreeMap, BTreeSet};

use rayon::prelude::*;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::read::BoundedReader;
use crate::walk::{FileClass, FileEntry, FileInventory};

/// Manifests above this size are not parsed.
const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    Npm,
    Pypi,
    Go,
    Cargo,
    Maven,
    Gradle,
    Rubygems,
    Composer,
    Nuget,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DepKind {
    Prod,
    Dev,
    Peer,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DepSpec {
    pub range: String,
    pub kind: DepKind,
    pub workspace_protocol: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ParseStatusLite {
    Ok,
    Partial,
    Failed { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModuleType {
    Module,
    Commonjs,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct NpmManifestExtras {
    pub module_type: Option<ModuleType>,
    pub main: Option<String>,
    pub module: Option<String>,
    pub types: Option<String>,
    pub bin: BTreeMap<String, String>,
    /// Kept verbatim, at most 64 KiB serialized.
    pub exports: Option<serde_json::Value>,
    pub workspaces: Option<Vec<String>>,
    pub script_names: Vec<String>,
    /// Path-like tokens of script values only. Raw commands are dropped: they can embed secrets.
    pub script_entry_hints: BTreeMap<String, Vec<String>>,
    pub engines: BTreeMap<String, String>,
    /// The `packageManager` field, e.g. `pnpm@10.4.1`.
    pub package_manager: Option<String>,
    /// True when package.json has a top-level `jest` key.
    pub has_jest_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ManifestFact {
    pub path: RepoPath,
    pub ecosystem: Ecosystem,
    pub name: Option<String>,
    pub version: Option<String>,
    pub private: bool,
    pub dependencies: BTreeMap<String, DepSpec>,
    pub npm: Option<NpmManifestExtras>,
    /// Maven `modules`, Gradle `include`s and similar child declarations.
    pub members: Vec<String>,
    /// Small ecosystem-specific facts (go version, python tool, ...), never free text from the
    /// repository beyond identifiers.
    pub meta: BTreeMap<String, String>,
    pub parse_status: ParseStatusLite,
}

impl ManifestFact {
    pub(crate) fn new(path: RepoPath, ecosystem: Ecosystem) -> Self {
        Self {
            path,
            ecosystem,
            name: None,
            version: None,
            private: false,
            dependencies: BTreeMap::new(),
            npm: None,
            members: Vec::new(),
            meta: BTreeMap::new(),
            parse_status: ParseStatusLite::Ok,
        }
    }

    pub(crate) fn failed(path: RepoPath, ecosystem: Ecosystem, reason: impl Into<String>) -> Self {
        let mut fact = Self::new(path, ecosystem);
        fact.parse_status = ParseStatusLite::Failed {
            reason: reason.into(),
        };
        fact
    }

    pub fn dir(&self) -> RepoDir {
        RepoDir::parent_of(&self.path)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PackageManagerKind {
    Npm,
    Pnpm,
    YarnClassic,
    YarnBerry,
    Bun,
    Pip,
    Poetry,
    Uv,
    GoModules,
    Cargo,
    Maven,
    Gradle,
    Bundler,
    Composer,
    Nuget,
}

impl PackageManagerKind {
    /// Priority among JavaScript package managers when several lockfiles are present.
    fn js_priority(self) -> Option<u8> {
        match self {
            Self::Pnpm => Some(0),
            Self::YarnBerry | Self::YarnClassic => Some(1),
            Self::Bun => Some(2),
            Self::Npm => Some(3),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PackageManagerFact {
    pub kind: PackageManagerKind,
    pub lockfile: Option<RepoPath>,
    pub lockfile_version: Option<String>,
    /// The `packageManager` field, e.g. `pnpm@10.4.1`.
    pub declared: Option<String>,
    pub scope_dir: RepoDir,
    /// The manager that wins in its directory (declared, single lockfile, or priority).
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ManifestFacts {
    pub manifests: Vec<ManifestFact>,
    pub package_managers: Vec<PackageManagerFact>,
}

impl ManifestFacts {
    pub fn manifest(&self, path: &str) -> Option<&ManifestFact> {
        self.manifests.iter().find(|m| m.path.as_str() == path)
    }

    pub fn npm_manifest_in(&self, dir: &RepoDir) -> Option<&ManifestFact> {
        self.manifests
            .iter()
            .find(|m| m.ecosystem == Ecosystem::Npm && m.dir() == *dir)
    }
}

fn basename(path: &RepoPath) -> &str {
    path.as_str().rsplit('/').next().unwrap_or(path.as_str())
}

fn manifest_ecosystem(name: &str) -> Option<Ecosystem> {
    let lower = name.to_ascii_lowercase();
    if lower == "package.json" {
        return Some(Ecosystem::Npm);
    }
    if lower == "pyproject.toml"
        || lower == "pipfile"
        || (lower.starts_with("requirements") && lower.ends_with(".txt"))
    {
        return Some(Ecosystem::Pypi);
    }
    match lower.as_str() {
        "go.mod" => return Some(Ecosystem::Go),
        "cargo.toml" => return Some(Ecosystem::Cargo),
        "pom.xml" => return Some(Ecosystem::Maven),
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => {
            return Some(Ecosystem::Gradle)
        }
        "gemfile" => return Some(Ecosystem::Rubygems),
        "composer.json" => return Some(Ecosystem::Composer),
        _ => {}
    }
    if lower.ends_with(".csproj") {
        return Some(Ecosystem::Nuget);
    }
    None
}

fn parse_manifest(
    ecosystem: Ecosystem,
    path: &RepoPath,
    text: &str,
) -> (ManifestFact, Vec<InitWarning>) {
    let name = basename(path).to_ascii_lowercase();
    match ecosystem {
        Ecosystem::Npm => npm::parse(path, text),
        Ecosystem::Pypi => python::parse(path, &name, text),
        Ecosystem::Go => go::parse(path, text),
        Ecosystem::Cargo => cargo::parse(path, text),
        Ecosystem::Maven => jvm::parse_pom(path, text),
        Ecosystem::Gradle => jvm::parse_gradle(path, &name, text),
        Ecosystem::Rubygems => other::parse_gemfile(path, text),
        Ecosystem::Composer => other::parse_composer(path, text),
        Ecosystem::Nuget => other::parse_csproj(path, text),
    }
}

/// Lockfile names and the manager each one implies.
fn lockfile_kind(name: &str) -> Option<PackageManagerKind> {
    match name {
        "package-lock.json" | "npm-shrinkwrap.json" => Some(PackageManagerKind::Npm),
        "pnpm-lock.yaml" => Some(PackageManagerKind::Pnpm),
        "yarn.lock" => Some(PackageManagerKind::YarnClassic),
        "bun.lockb" | "bun.lock" => Some(PackageManagerKind::Bun),
        "uv.lock" => Some(PackageManagerKind::Uv),
        "poetry.lock" => Some(PackageManagerKind::Poetry),
        _ => None,
    }
}

fn lockfile_version(
    reader: &BoundedReader,
    entry: &FileEntry,
    kind: PackageManagerKind,
    name: &str,
) -> (PackageManagerKind, Option<String>) {
    // bun.lockb is binary and bun.lock is JSONC: presence is enough.
    if name == "bun.lockb" || name == "bun.lock" {
        return (kind, None);
    }
    let Ok(text) = reader.read_text(entry, 4096) else {
        return (kind, None);
    };
    match kind {
        PackageManagerKind::Npm => (kind, capture(r#""lockfileVersion"\s*:\s*(\d+)"#, &text)),
        PackageManagerKind::Pnpm => {
            let first = text.lines().next().unwrap_or_default();
            (kind, capture(r#"lockfileVersion:\s*['"]?([\d.]+)"#, first))
        }
        PackageManagerKind::YarnClassic => {
            if text.contains("__metadata:") {
                (
                    PackageManagerKind::YarnBerry,
                    capture(r"__metadata:\s*\n\s*version:\s*(\d+)", &text),
                )
            } else {
                (
                    kind,
                    text.contains("yarn lockfile v1").then_some("1".to_owned()),
                )
            }
        }
        _ => (kind, None),
    }
}

fn capture(pattern: &str, text: &str) -> Option<String> {
    regex::Regex::new(pattern)
        .ok()?
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
}

fn declared_kind(declared: &str) -> Option<PackageManagerKind> {
    let (name, version) = declared.split_once('@').unwrap_or((declared, ""));
    match name {
        "npm" => Some(PackageManagerKind::Npm),
        "pnpm" => Some(PackageManagerKind::Pnpm),
        "yarn" => {
            let major: u32 = version
                .split('.')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            Some(if major >= 2 {
                PackageManagerKind::YarnBerry
            } else {
                PackageManagerKind::YarnClassic
            })
        }
        "bun" => Some(PackageManagerKind::Bun),
        _ => None,
    }
}

/// Detects every manifest and package manager in the inventory.
pub fn detect_manifests(
    inventory: &FileInventory,
    reader: &BoundedReader,
) -> (ManifestFacts, Vec<InitWarning>) {
    let span = tracing::info_span!(
        "init.manifests",
        manifests.count = tracing::field::Empty,
        package_manager.primary = tracing::field::Empty,
        lockfiles.count = tracing::field::Empty
    );
    let _guard = span.enter();

    let candidates: Vec<(&FileEntry, Ecosystem)> = inventory
        .entries
        .iter()
        .filter_map(|e| manifest_ecosystem(basename(&e.path)).map(|eco| (e, eco)))
        .collect();

    let parsed: Vec<(ManifestFact, Vec<InitWarning>)> = candidates
        .par_iter()
        .map(|(entry, ecosystem)| {
            if entry.class == FileClass::TooLarge || entry.size as usize > MAX_MANIFEST_BYTES {
                let warning = InitWarning::new(
                    "manifest_too_large",
                    Some(entry.path.clone()),
                    "manifest is too large to parse",
                );
                return (
                    ManifestFact::failed(entry.path.clone(), *ecosystem, "too large"),
                    vec![warning],
                );
            }
            match reader.read_text(entry, MAX_MANIFEST_BYTES) {
                Ok(text) => parse_manifest(*ecosystem, &entry.path, &text),
                Err(_) => {
                    let warning = InitWarning::new(
                        "manifest_parse",
                        Some(entry.path.clone()),
                        "manifest could not be read",
                    );
                    (
                        ManifestFact::failed(entry.path.clone(), *ecosystem, "unreadable"),
                        vec![warning],
                    )
                }
            }
        })
        .collect();

    let mut manifests = Vec::new();
    let mut warnings = Vec::new();
    for (fact, w) in parsed {
        manifests.push(fact);
        warnings.extend(w);
    }
    manifests.sort_by(|a, b| a.path.cmp(&b.path));

    let (package_managers, pm_warnings) = package_managers(inventory, reader, &manifests);
    warnings.extend(pm_warnings);
    warnings.sort();

    span.record("manifests.count", manifests.len());
    span.record(
        "lockfiles.count",
        package_managers
            .iter()
            .filter(|p| p.lockfile.is_some())
            .count(),
    );
    if let Some(primary) = package_managers.iter().find(|p| p.primary) {
        span.record("package_manager.primary", format!("{:?}", primary.kind));
    }
    (
        ManifestFacts {
            manifests,
            package_managers,
        },
        warnings,
    )
}

fn package_managers(
    inventory: &FileInventory,
    reader: &BoundedReader,
    manifests: &[ManifestFact],
) -> (Vec<PackageManagerFact>, Vec<InitWarning>) {
    let mut warnings = Vec::new();
    // scope dir -> candidate facts
    let mut by_dir: BTreeMap<RepoDir, Vec<PackageManagerFact>> = BTreeMap::new();

    for entry in &inventory.entries {
        let name = basename(&entry.path);
        let Some(kind) = lockfile_kind(name) else {
            continue;
        };
        let (kind, version) = lockfile_version(reader, entry, kind, name);
        by_dir
            .entry(RepoDir::parent_of(&entry.path))
            .or_default()
            .push(PackageManagerFact {
                kind,
                lockfile: Some(entry.path.clone()),
                lockfile_version: version,
                declared: None,
                scope_dir: RepoDir::parent_of(&entry.path),
                primary: false,
            });
    }

    for manifest in manifests {
        let dir = manifest.dir();
        match manifest.ecosystem {
            Ecosystem::Npm => {
                let declared = manifest
                    .npm
                    .as_ref()
                    .and_then(|n| n.package_manager.clone());
                if let Some(declared) = declared {
                    if let Some(kind) = declared_kind(&declared) {
                        let facts = by_dir.entry(dir.clone()).or_default();
                        let same_family = |k: PackageManagerKind| {
                            k == kind
                                || matches!(
                                    (k, kind),
                                    (
                                        PackageManagerKind::YarnClassic,
                                        PackageManagerKind::YarnBerry
                                    ) | (
                                        PackageManagerKind::YarnBerry,
                                        PackageManagerKind::YarnClassic
                                    )
                                )
                        };
                        if let Some(existing) = facts.iter_mut().find(|f| same_family(f.kind)) {
                            existing.declared = Some(declared);
                            existing.kind = kind;
                        } else {
                            facts.push(PackageManagerFact {
                                kind,
                                lockfile: None,
                                lockfile_version: None,
                                declared: Some(declared),
                                scope_dir: dir,
                                primary: false,
                            });
                        }
                    }
                }
            }
            Ecosystem::Pypi => {
                let name = basename(&manifest.path).to_ascii_lowercase();
                let kind = if manifest.meta.contains_key("tool_poetry") {
                    PackageManagerKind::Poetry
                } else if name == "pyproject.toml"
                    && by_dir
                        .get(&dir)
                        .is_some_and(|f| f.iter().any(|p| p.kind == PackageManagerKind::Uv))
                {
                    PackageManagerKind::Uv
                } else {
                    PackageManagerKind::Pip
                };
                let facts = by_dir.entry(dir.clone()).or_default();
                if !facts.iter().any(|f| f.kind == kind) {
                    facts.push(PackageManagerFact {
                        kind,
                        lockfile: None,
                        lockfile_version: None,
                        declared: None,
                        scope_dir: dir,
                        primary: false,
                    });
                }
            }
            eco => {
                let kind = match eco {
                    Ecosystem::Go => PackageManagerKind::GoModules,
                    Ecosystem::Cargo => PackageManagerKind::Cargo,
                    Ecosystem::Maven => PackageManagerKind::Maven,
                    Ecosystem::Gradle => PackageManagerKind::Gradle,
                    Ecosystem::Rubygems => PackageManagerKind::Bundler,
                    Ecosystem::Composer => PackageManagerKind::Composer,
                    Ecosystem::Nuget => PackageManagerKind::Nuget,
                    _ => continue,
                };
                let facts = by_dir.entry(dir.clone()).or_default();
                if !facts.iter().any(|f| f.kind == kind) {
                    facts.push(PackageManagerFact {
                        kind,
                        lockfile: None,
                        lockfile_version: None,
                        declared: None,
                        scope_dir: dir,
                        primary: false,
                    });
                }
            }
        }
    }

    // Attach Cargo.lock to Cargo facts.
    for entry in &inventory.entries {
        if basename(&entry.path) == "Cargo.lock" {
            if let Some(facts) = by_dir.get_mut(&RepoDir::parent_of(&entry.path)) {
                for fact in facts
                    .iter_mut()
                    .filter(|f| f.kind == PackageManagerKind::Cargo)
                {
                    fact.lockfile = Some(entry.path.clone());
                }
            }
        }
    }

    let mut out = Vec::new();
    for (dir, mut facts) in by_dir {
        facts.sort_by_key(|f| (f.kind.js_priority().unwrap_or(9), f.kind));
        let js: Vec<usize> = facts
            .iter()
            .enumerate()
            .filter(|(_, f)| f.kind.js_priority().is_some())
            .map(|(i, _)| i)
            .collect();
        if js.len() > 1 {
            warnings.push(InitWarning::new(
                "multiple_lockfiles",
                facts[js[0]].lockfile.clone(),
                format!(
                    "{} package managers are present in `{}`",
                    js.len(),
                    if dir.is_root() { "." } else { dir.as_str() }
                ),
            ));
        }
        // The declared manager wins; otherwise the best priority.
        let primary_js = js
            .iter()
            .copied()
            .find(|&i| facts[i].declared.is_some())
            .or_else(|| js.first().copied());
        let non_js: BTreeSet<PackageManagerKind> = facts
            .iter()
            .filter(|f| f.kind.js_priority().is_none())
            .map(|f| f.kind)
            .collect();
        for (i, fact) in facts.iter_mut().enumerate() {
            fact.primary = if fact.kind.js_priority().is_some() {
                Some(i) == primary_js
            } else {
                non_js.contains(&fact.kind)
            };
        }
        out.extend(facts);
    }
    out.sort_by(|a, b| {
        (a.scope_dir.depth(), &a.scope_dir, !a.primary, a.kind).cmp(&(
            b.scope_dir.depth(),
            &b.scope_dir,
            !b.primary,
            b.kind,
        ))
    });
    (out, warnings)
}
