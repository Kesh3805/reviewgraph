//! Python module resolution: dotted specifiers to files, over a known set of repository files.
//!
//! Handles relative imports (`.models`, `..core.db`), absolute imports from the repository root or
//! a source root (`src/` layouts), packages (`pkg/__init__.py`) and plain modules (`pkg/mod.py`).
//! Anything else is a standard-library module or an external package. Nothing is imported or
//! executed.

use std::collections::BTreeSet;

use analysis_ir::{ModuleResolver, Resolution, ResolutionMethod, ResolveKind, UnresolvedReason};
use review_core::location::RepoPath;

/// Top-level standard library modules that commonly appear in imports.
const STDLIB: &[&str] = &[
    "__future__",
    "abc",
    "argparse",
    "asyncio",
    "base64",
    "collections",
    "contextlib",
    "copy",
    "csv",
    "dataclasses",
    "datetime",
    "decimal",
    "enum",
    "functools",
    "hashlib",
    "http",
    "importlib",
    "inspect",
    "io",
    "itertools",
    "json",
    "logging",
    "math",
    "multiprocessing",
    "operator",
    "os",
    "pathlib",
    "pickle",
    "queue",
    "random",
    "re",
    "secrets",
    "shutil",
    "socket",
    "sqlite3",
    "string",
    "subprocess",
    "sys",
    "tempfile",
    "threading",
    "time",
    "traceback",
    "types",
    "typing",
    "unittest",
    "urllib",
    "uuid",
    "warnings",
    "weakref",
    "xml",
    "zipfile",
];

/// Whether a top-level module name belongs to the standard library.
pub fn is_stdlib(top: &str) -> bool {
    STDLIB.contains(&top)
}

/// Resolves Python import specifiers against a set of repository files.
#[derive(Debug, Clone)]
pub struct PythonModuleResolver {
    files: BTreeSet<String>,
    roots: Vec<String>,
}

impl PythonModuleResolver {
    /// `source_roots` are repository-relative directories searched for absolute imports, in order
    /// (the empty string is the repository root). An empty list means the root and `src`.
    pub fn new(files: impl IntoIterator<Item = RepoPath>, source_roots: Vec<String>) -> Self {
        let roots = if source_roots.is_empty() {
            vec![String::new(), "src".to_owned()]
        } else {
            source_roots
        };
        Self {
            files: files.into_iter().map(|p| p.as_str().to_owned()).collect(),
            roots,
        }
    }

    fn join(base: &str, rest: &str) -> String {
        match (base.is_empty(), rest.is_empty()) {
            (true, _) => rest.to_owned(),
            (false, true) => base.to_owned(),
            (false, false) => format!("{base}/{rest}"),
        }
    }

    /// Looks for `<base>/<dotted>.py` or `<base>/<dotted>/__init__.py`.
    fn find(
        &self,
        base: &str,
        dotted: &str,
        method: ResolutionMethod,
        confidence: f32,
    ) -> Option<Resolution> {
        let path = Self::join(base, &dotted.replace('.', "/"));
        let module = format!("{path}.py");
        let package = Self::join(&path, "__init__.py");
        let (candidate, method) = if !dotted.is_empty() && self.files.contains(&module) {
            (module, method)
        } else if self.files.contains(&package) {
            (package, ResolutionMethod::Index)
        } else {
            return None;
        };
        Some(Resolution::File {
            path: RepoPath::new(candidate).ok()?,
            method,
            confidence,
        })
    }
}

impl ModuleResolver for PythonModuleResolver {
    fn resolve(&self, from: &RepoPath, specifier: &str, kind: ResolveKind) -> Resolution {
        if kind == ResolveKind::Dynamic && specifier.is_empty() {
            return Resolution::Unresolved {
                reason: UnresolvedReason::NonLiteral,
            };
        }
        let dots = specifier.chars().take_while(|c| *c == '.').count();
        let rest = &specifier[dots..];
        if dots > 0 {
            let mut base: Vec<&str> = from.as_str().split('/').collect();
            base.pop();
            for _ in 1..dots {
                if base.pop().is_none() {
                    return Resolution::Unresolved {
                        reason: UnresolvedReason::OutsideRepository,
                    };
                }
            }
            return self
                .find(&base.join("/"), rest, ResolutionMethod::Relative, 1.0)
                .unwrap_or(Resolution::Unresolved {
                    reason: UnresolvedReason::NotFound,
                });
        }
        if rest.is_empty() {
            return Resolution::Unresolved {
                reason: UnresolvedReason::Unsupported,
            };
        }
        for root in &self.roots {
            if let Some(found) = self.find(root, rest, ResolutionMethod::BaseUrl, 0.95) {
                return found;
            }
        }
        let mut segments = rest.split('.');
        let top = segments.next().unwrap_or(rest);
        if is_stdlib(top) {
            return Resolution::Builtin {
                name: top.to_owned(),
            };
        }
        let subpath: Vec<&str> = segments.collect();
        Resolution::External {
            ecosystem: "pypi".to_owned(),
            name: top.to_owned(),
            subpath: if subpath.is_empty() {
                None
            } else {
                Some(subpath.join("/"))
            },
            version_range: None,
        }
    }
}
