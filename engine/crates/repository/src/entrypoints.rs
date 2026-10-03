//! Process entrypoints: HTTP servers, workers, CLIs, scripts, libraries (INIT-009).
//!
//! Sources, strongest first: package.json `main`/`bin`/scripts, `nest-cli.json`, bootstrap calls
//! found by a text scan, Dockerfile/Procfile/serverless handlers, and conventional file names.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use rayon::prelude::*;
use regex::Regex;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::manifests::{script_entry_hints, Ecosystem, ManifestFacts};
use crate::read::BoundedReader;
use crate::tsconfig::{join_normalized, TsConfigSet};
use crate::walk::{FileClass, FileInventory};

const MAX_SCAN_BYTES: u64 = 256 * 1024;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EntrypointKind {
    HttpServer,
    Worker,
    Microservice,
    Cli,
    Script,
    Library,
    Serverless,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EntrySource {
    PackageMain,
    PackageBin { name: String },
    Script { name: String },
    NestCli,
    Bootstrap { call: String },
    Conventional,
    Dockerfile,
    Procfile,
    Serverless,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EntrypointFact {
    pub path: RepoPath,
    pub kind: EntrypointKind,
    #[serde(flatten)]
    pub source: EntrySource,
    pub confidence: f32,
    pub package: Option<String>,
}

fn is_code(path: &str) -> bool {
    [".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"]
        .iter()
        .any(|e| path.ends_with(e))
}

fn worker_word(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ["worker", "queue", "consumer"]
        .iter()
        .any(|w| lower.contains(w))
}

struct Resolver<'a> {
    inv: &'a FileInventory,
    ts: &'a TsConfigSet,
}

impl Resolver<'_> {
    fn exists(&self, path: &str) -> bool {
        self.inv.find(path).is_some()
    }

    /// Maps a build-output hint (`dist/main`, `./dist/main.js`) relative to `dir` back to the
    /// source file in the inventory.
    fn dist_to_src(&self, dir: &RepoDir, hint: &str) -> Option<RepoPath> {
        let hint = hint.trim_start_matches("./");
        let joined = join_normalized(dir, hint)?;
        let joined = joined.as_str();
        let stem = |p: &str| -> String {
            for ext in [".js", ".mjs", ".cjs", ".d.ts", ".ts", ".tsx", ".jsx"] {
                if let Some(s) = p.strip_suffix(ext) {
                    return s.to_owned();
                }
            }
            p.to_owned()
        };
        // Owning tsconfig in the package directory, then at the root.
        let config = [dir.clone(), RepoDir::root()].into_iter().find_map(|d| {
            let name = d.join("tsconfig.json").ok()?;
            self.ts.get(name.as_str())
        });
        let mut candidates: Vec<String> = Vec::new();
        if let Some(c) = config {
            let out = c.effective.out_dir.as_ref();
            let root = c.effective.root_dir.clone().or_else(|| {
                RepoDir::new(if dir.is_root() {
                    "src".to_owned()
                } else {
                    format!("{dir}/src")
                })
                .ok()
            });
            if let (Some(out), Some(root)) = (out, root) {
                if let Some(rest) = joined.strip_prefix(&format!("{out}/")) {
                    let base = if root.is_root() {
                        stem(rest)
                    } else {
                        format!("{root}/{}", stem(rest))
                    };
                    candidates.push(base);
                }
            }
        }
        // Conventional output directory names.
        for out in ["dist", "build", "lib", "out"] {
            let prefix = if dir.is_root() {
                format!("{out}/")
            } else {
                format!("{dir}/{out}/")
            };
            if let Some(rest) = joined.strip_prefix(&prefix) {
                let src = if dir.is_root() {
                    "src".to_owned()
                } else {
                    format!("{dir}/src")
                };
                candidates.push(format!("{src}/{}", stem(rest)));
            }
        }
        candidates.push(stem(joined));
        for base in candidates {
            for ext in [".ts", ".tsx", ".js", ".mjs", ".cjs", ".jsx"] {
                let candidate = format!("{base}{ext}");
                if self.exists(&candidate) {
                    return RepoPath::new(candidate).ok();
                }
            }
        }
        None
    }
}

fn bootstrap_needles() -> &'static [(&'static str, EntrypointKind, f32)] {
    &[
        ("NestFactory.create(", EntrypointKind::HttpServer, 0.9),
        ("NestFactory.create<", EntrypointKind::HttpServer, 0.9),
        (
            "NestFactory.createApplicationContext(",
            EntrypointKind::Worker,
            0.8,
        ),
        (
            "NestFactory.createMicroservice(",
            EntrypointKind::Microservice,
            0.9,
        ),
    ]
}

fn dockerfile_regex() -> &'static Option<Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?im)^\s*(?:CMD|ENTRYPOINT)\s+(.+)$").ok())
}

fn handler_regex() -> &'static Option<Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"handler:\s*['"]?([^\s'"]+)"#).ok())
}

/// Detects process entrypoints.
pub fn detect_entrypoints(
    inv: &FileInventory,
    reader: &BoundedReader,
    manifests: &ManifestFacts,
    ts: &TsConfigSet,
) -> (Vec<EntrypointFact>, Vec<InitWarning>) {
    let span = tracing::info_span!(
        "init.entrypoints",
        entrypoints.count = tracing::field::Empty
    );
    let _guard = span.enter();

    let resolver = Resolver { inv, ts };
    let mut found: Vec<EntrypointFact> = Vec::new();
    let npm_dirs: Vec<(RepoDir, Option<String>)> = manifests
        .manifests
        .iter()
        .filter(|m| m.ecosystem == Ecosystem::Npm)
        .map(|m| (m.dir(), m.name.clone()))
        .collect();
    let package_of = |path: &str| -> Option<String> {
        npm_dirs
            .iter()
            .filter(|(d, _)| d.contains(path))
            .max_by_key(|(d, _)| d.depth())
            .and_then(|(_, n)| n.clone())
    };
    let push = |found: &mut Vec<EntrypointFact>,
                path: RepoPath,
                kind: EntrypointKind,
                source: EntrySource,
                confidence: f32| {
        let package = package_of(path.as_str());
        found.push(EntrypointFact {
            path,
            kind,
            source,
            confidence,
            package,
        });
    };

    // package.json main / bin / exports["."] / script hints.
    for manifest in manifests
        .manifests
        .iter()
        .filter(|m| m.ecosystem == Ecosystem::Npm)
    {
        let Some(extras) = &manifest.npm else {
            continue;
        };
        let dir = manifest.dir();
        let mut mains: Vec<String> = Vec::new();
        mains.extend(extras.main.clone());
        if let Some(exports) = &extras.exports {
            let dot = exports.get(".").unwrap_or(exports);
            match dot {
                serde_json::Value::String(s) => mains.push(s.clone()),
                serde_json::Value::Object(o) => {
                    for key in ["import", "require", "default", "node"] {
                        if let Some(s) = o.get(key).and_then(|v| v.as_str()) {
                            mains.push(s.to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
        for hint in mains {
            if let Some(path) = resolver.dist_to_src(&dir, &hint) {
                push(
                    &mut found,
                    path,
                    EntrypointKind::Library,
                    EntrySource::PackageMain,
                    0.8,
                );
            }
        }
        for (name, hint) in &extras.bin {
            if let Some(path) = resolver.dist_to_src(&dir, hint) {
                push(
                    &mut found,
                    path,
                    EntrypointKind::Cli,
                    EntrySource::PackageBin { name: name.clone() },
                    0.9,
                );
            }
        }
        for (name, hints) in &extras.script_entry_hints {
            for hint in hints {
                let Some(path) = resolver.dist_to_src(&dir, hint) else {
                    continue;
                };
                let kind = if worker_word(path.as_str()) || worker_word(name) {
                    EntrypointKind::Worker
                } else if name.starts_with("start") || name.starts_with("serve") || name == "dev" {
                    EntrypointKind::HttpServer
                } else {
                    EntrypointKind::Script
                };
                push(
                    &mut found,
                    path,
                    kind,
                    EntrySource::Script { name: name.clone() },
                    0.6,
                );
            }
        }
    }

    // nest-cli.json
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
        let mut projects: Vec<(Option<String>, Option<String>)> = vec![(
            value
                .get("sourceRoot")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            value
                .get("entryFile")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        )];
        if let Some(map) = value.get("projects").and_then(|v| v.as_object()) {
            for p in map.values() {
                projects.push((
                    p.get("sourceRoot")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                    p.get("entryFile")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                ));
            }
        }
        for (source_root, entry_file) in projects {
            let Some(root) = source_root else { continue };
            let file = entry_file.unwrap_or_else(|| "main".to_owned());
            let Some(base) = join_normalized(&dir, &root) else {
                continue;
            };
            for ext in [".ts", ".js"] {
                let Ok(path) = base.join(&format!("{file}{ext}")) else {
                    continue;
                };
                if inv.find(path.as_str()).is_some() {
                    let kind = if worker_word(path.as_str()) {
                        EntrypointKind::Worker
                    } else {
                        EntrypointKind::HttpServer
                    };
                    push(&mut found, path, kind, EntrySource::NestCli, 0.85);
                    break;
                }
            }
        }
    }

    // Bootstrap text scan.
    let scan: Vec<EntrypointFact> = inv
        .entries
        .par_iter()
        .filter(|e| {
            e.class == FileClass::Source && e.size <= MAX_SCAN_BYTES && is_code(e.path.as_str())
        })
        .flat_map_iter(|entry| {
            let mut hits = Vec::new();
            let Ok(text) = reader.read_text(entry, MAX_SCAN_BYTES as usize) else {
                return Vec::new();
            };
            for (needle, kind, confidence) in bootstrap_needles() {
                if text.contains(needle) {
                    hits.push((
                        *kind,
                        needle.trim_end_matches(['(', '<']).to_owned(),
                        *confidence,
                    ));
                }
            }
            if text.contains("new Worker(") && text.contains("bullmq") {
                hits.push((EntrypointKind::Worker, "new Worker".to_owned(), 0.8));
            }
            if text.contains(".listen(") && text.contains("express()") {
                hits.push((EntrypointKind::HttpServer, "express.listen".to_owned(), 0.7));
            }
            if text.starts_with("#!") && entry.path.as_str().split('/').any(|s| s == "bin") {
                hits.push((EntrypointKind::Cli, "shebang".to_owned(), 0.9));
            }
            hits.into_iter()
                .map(|(kind, call, confidence)| EntrypointFact {
                    path: entry.path.clone(),
                    kind,
                    source: EntrySource::Bootstrap { call },
                    confidence,
                    package: None,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for fact in scan {
        push(
            &mut found,
            fact.path,
            fact.kind,
            fact.source,
            fact.confidence,
        );
    }

    // scripts/**
    for entry in &inv.entries {
        let path = entry.path.as_str();
        if (path.starts_with("scripts/") || path.contains("/scripts/")) && is_code(path) {
            push(
                &mut found,
                entry.path.clone(),
                EntrypointKind::Script,
                EntrySource::Conventional,
                0.6,
            );
        }
    }

    // Dockerfile CMD / ENTRYPOINT, Procfile, serverless handlers.
    for entry in &inv.entries {
        let path = entry.path.as_str();
        let name = path.rsplit('/').next().unwrap_or(path);
        let dir = RepoDir::parent_of(&entry.path);
        if name == "Dockerfile" || name.starts_with("Dockerfile.") {
            let (Ok(text), Some(re)) = (reader.read_text(entry, 64 * 1024), dockerfile_regex())
            else {
                continue;
            };
            for caps in re.captures_iter(&text) {
                let line = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                for hint in script_entry_hints(&line.replace(['[', ']', ',', '"'], " ")) {
                    if let Some(src) = resolver.dist_to_src(&dir, &hint) {
                        let kind = if worker_word(src.as_str()) {
                            EntrypointKind::Worker
                        } else {
                            EntrypointKind::HttpServer
                        };
                        push(&mut found, src, kind, EntrySource::Dockerfile, 0.7);
                    }
                }
            }
        } else if name == "Procfile" {
            let Ok(text) = reader.read_text(entry, 16 * 1024) else {
                continue;
            };
            for line in text.lines() {
                let Some((process, command)) = line.split_once(':') else {
                    continue;
                };
                for hint in script_entry_hints(command) {
                    if let Some(src) = resolver.dist_to_src(&dir, &hint) {
                        let kind = if worker_word(process) || worker_word(src.as_str()) {
                            EntrypointKind::Worker
                        } else {
                            EntrypointKind::HttpServer
                        };
                        push(&mut found, src, kind, EntrySource::Procfile, 0.7);
                    }
                }
            }
        } else if matches!(name, "serverless.yml" | "serverless.yaml") {
            let (Ok(text), Some(re)) = (reader.read_text(entry, 128 * 1024), handler_regex())
            else {
                continue;
            };
            for caps in re.captures_iter(&text) {
                let handler = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                let file = handler.rsplit_once('.').map(|(f, _)| f).unwrap_or(handler);
                let base = dir.join(file.trim_start_matches("./")).ok();
                let Some(base) = base else { continue };
                for ext in [".ts", ".js", ".mjs"] {
                    let candidate = format!("{}{ext}", base.as_str());
                    if inv.find(&candidate).is_some() {
                        if let Ok(path) = RepoPath::new(candidate) {
                            push(
                                &mut found,
                                path,
                                EntrypointKind::Serverless,
                                EntrySource::Serverless,
                                0.8,
                            );
                        }
                        break;
                    }
                }
            }
        }
    }

    // Conventional files, only for package scopes that have no stronger entrypoint.
    let strong_dirs: BTreeSet<RepoDir> = found
        .iter()
        .filter(|f| f.confidence > 0.5 && f.kind != EntrypointKind::Script)
        .map(|f| {
            npm_dirs
                .iter()
                .filter(|(d, _)| d.contains(f.path.as_str()))
                .map(|(d, _)| d.clone())
                .max_by_key(RepoDir::depth)
                .unwrap_or_default()
        })
        .collect();
    let mut scopes: BTreeSet<RepoDir> = npm_dirs.iter().map(|(d, _)| d.clone()).collect();
    scopes.insert(RepoDir::root());
    for scope in scopes {
        if strong_dirs.contains(&scope) {
            continue;
        }
        for candidate in [
            "src/main.ts",
            "src/index.ts",
            "src/server.ts",
            "src/app.ts",
            "index.ts",
            "index.js",
        ] {
            let Ok(path) = scope.join(candidate) else {
                continue;
            };
            if inv.find(path.as_str()).is_some() {
                let kind = if candidate.ends_with("main.ts") || candidate.ends_with("server.ts") {
                    EntrypointKind::HttpServer
                } else {
                    EntrypointKind::Library
                };
                push(&mut found, path, kind, EntrySource::Conventional, 0.5);
            }
        }
    }

    // Deduplicate by (path, kind) keeping the strongest evidence.
    let mut best: BTreeMap<(RepoPath, EntrypointKind), EntrypointFact> = BTreeMap::new();
    for fact in found {
        let key = (fact.path.clone(), fact.kind);
        match best.get(&key) {
            Some(existing) if existing.confidence >= fact.confidence => {}
            _ => {
                best.insert(key, fact);
            }
        }
    }
    let out: Vec<EntrypointFact> = best.into_values().collect();
    span.record("entrypoints.count", out.len());
    (out, Vec::new())
}
