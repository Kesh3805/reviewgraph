//! Helpers for the INC-002.. tests: in-memory TypeScript repositories, a relative-import resolver,
//! a counting parser and a full-build shortcut.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use analysis_ir::traits::UnresolvedReason;
use analysis_ir::traits::{ModuleResolver, Resolution, ResolutionMethod, ResolveKind};
use analysis_ir::ParsedUnit;
use codegraph::{Graph, LinkConfig};
use incremental::full::build_full_graph;
use incremental::reparse::UnitParser;
use review_core::location::{ContentHash, RepoPath};

/// Resolves `./x` and `../x` against a fixed file set; bare specifiers are external packages.
#[derive(Debug, Clone, Default)]
pub struct RelativeResolver {
    files: BTreeSet<String>,
}

impl RelativeResolver {
    pub fn new<'a>(files: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            files: files.into_iter().map(str::to_owned).collect(),
        }
    }
}

fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

impl ModuleResolver for RelativeResolver {
    fn resolve(&self, from: &RepoPath, specifier: &str, _kind: ResolveKind) -> Resolution {
        if specifier.starts_with("./") || specifier.starts_with("../") {
            let dir = from.as_str().rsplit_once('/').map_or("", |(d, _)| d);
            let joined = normalize(&format!("{dir}/{specifier}"));
            for candidate in [
                format!("{joined}.ts"),
                format!("{joined}/index.ts"),
                joined.clone(),
            ] {
                if self.files.contains(&candidate) {
                    if let Ok(path) = RepoPath::new(candidate) {
                        return Resolution::File {
                            path,
                            method: ResolutionMethod::Relative,
                            confidence: 1.0,
                        };
                    }
                }
            }
            return Resolution::Unresolved {
                reason: UnresolvedReason::NotFound,
            };
        }
        Resolution::External {
            ecosystem: "npm".to_owned(),
            name: specifier.to_owned(),
            subpath: None,
            version_range: None,
        }
    }
}

/// Parses with the real TypeScript analyzer and counts every call, remembering the paths.
#[derive(Debug, Default)]
pub struct CountingParser {
    calls: AtomicUsize,
    parsed: Mutex<Vec<RepoPath>>,
}

impl CountingParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn parsed_paths(&self) -> Vec<RepoPath> {
        self.parsed.lock().unwrap().clone()
    }
}

impl UnitParser for CountingParser {
    fn supports(&self, path: &RepoPath) -> bool {
        path.as_str().ends_with(".ts")
    }

    fn analyzer_version(&self, _path: &RepoPath) -> String {
        "test-1".to_owned()
    }

    fn parse(
        &self,
        path: &RepoPath,
        bytes: &[u8],
        _content_hash: ContentHash,
    ) -> Result<ParsedUnit, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.parsed.lock().unwrap().push(path.clone());
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        Ok(super::analyze(path.as_str(), text))
    }
}

/// Parses every `(path, source)` pair into a unit.
pub fn units_of(files: &BTreeMap<String, String>) -> Vec<Arc<ParsedUnit>> {
    files
        .iter()
        .filter(|(path, _)| path.ends_with(".ts"))
        .map(|(path, source)| Arc::new(super::analyze(path, source)))
        .collect()
}

/// The full build of an in-memory repository.
pub fn full_graph(files: &BTreeMap<String, String>) -> Graph {
    let units = units_of(files);
    let resolver = RelativeResolver::new(files.keys().map(String::as_str));
    build_full_graph(&units, &resolver, &LinkConfig::default(), "fixture").unwrap()
}

/// A repository from literal pairs.
pub fn repo(files: &[(&str, &str)]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(p, s)| ((*p).to_owned(), (*s).to_owned()))
        .collect()
}
