//! A file-set module resolver (IDX-001).
//!
//! Resolves relative specifiers against the set of files the walk found (extension probing and
//! `index` files, `.js` specifiers mapping onto `.ts` sources) and treats every bare specifier as
//! an external npm package. tsconfig `paths` and workspace packages are not modelled yet, so
//! such imports surface as external packages rather than as edges.

use std::collections::BTreeSet;

use analysis_ir::traits::{
    ModuleResolver, Resolution, ResolutionMethod, ResolveKind, UnresolvedReason,
};
use review_core::location::RepoPath;

const EXTENSIONS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

const NODE_BUILTINS: [&str; 24] = [
    "assert",
    "buffer",
    "child_process",
    "cluster",
    "crypto",
    "dns",
    "events",
    "fs",
    "http",
    "https",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "querystring",
    "readline",
    "stream",
    "timers",
    "tls",
    "url",
    "util",
    "worker_threads",
    "zlib",
];

/// Resolves specifiers against a fixed set of repository files.
#[derive(Debug, Clone, Default)]
pub struct FileSetResolver {
    files: BTreeSet<String>,
}

impl FileSetResolver {
    pub fn new<'a>(paths: impl IntoIterator<Item = &'a RepoPath>) -> Self {
        Self {
            files: paths.into_iter().map(|p| p.as_str().to_owned()).collect(),
        }
    }

    fn probe(&self, base: &str) -> Option<(String, ResolutionMethod)> {
        if self.files.contains(base) {
            return Some((base.to_owned(), ResolutionMethod::Relative));
        }
        // `./a.js` written in TypeScript sources points at `./a.ts`.
        if let Some((stem, ext)) = base.rsplit_once('.') {
            if matches!(ext, "js" | "jsx" | "mjs" | "cjs") {
                for candidate in EXTENSIONS {
                    let path = format!("{stem}.{candidate}");
                    if self.files.contains(&path) {
                        return Some((path, ResolutionMethod::Relative));
                    }
                }
            }
        }
        for ext in EXTENSIONS {
            let path = format!("{base}.{ext}");
            if self.files.contains(&path) {
                return Some((path, ResolutionMethod::Relative));
            }
        }
        for ext in EXTENSIONS {
            let path = format!("{base}/index.{ext}");
            if self.files.contains(&path) {
                return Some((path, ResolutionMethod::Index));
            }
        }
        None
    }
}

/// `dir` joined with `specifier`, collapsing `.` and `..`; `None` when it leaves the root.
fn join_relative(dir: &str, specifier: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for segment in specifier.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// `@scope/name/sub` -> (`@scope/name`, `sub`); `name/sub` -> (`name`, `sub`).
fn split_package(specifier: &str) -> (String, Option<String>) {
    let mut it = specifier.splitn(if specifier.starts_with('@') { 3 } else { 2 }, '/');
    let first = it.next().unwrap_or(specifier);
    if specifier.starts_with('@') {
        let second = it.next().unwrap_or("");
        let name = format!("{first}/{second}");
        (name, it.next().map(str::to_owned))
    } else {
        (first.to_owned(), it.next().map(str::to_owned))
    }
}

impl ModuleResolver for FileSetResolver {
    fn resolve(&self, from: &RepoPath, specifier: &str, _kind: ResolveKind) -> Resolution {
        if specifier.starts_with('.') {
            let dir = from.as_str().rsplit_once('/').map_or("", |(d, _)| d);
            let Some(joined) = join_relative(dir, specifier) else {
                return Resolution::Unresolved {
                    reason: UnresolvedReason::OutsideRepository,
                };
            };
            return match self.probe(&joined) {
                Some((path, method)) => match RepoPath::new(path) {
                    Ok(path) => Resolution::File {
                        path,
                        method,
                        confidence: 1.0,
                    },
                    Err(_) => Resolution::Unresolved {
                        reason: UnresolvedReason::NotFound,
                    },
                },
                None => Resolution::Unresolved {
                    reason: UnresolvedReason::NotFound,
                },
            };
        }
        if let Some(name) = specifier.strip_prefix("node:") {
            return Resolution::Builtin {
                name: name.to_owned(),
            };
        }
        if specifier.is_empty() {
            return Resolution::Unresolved {
                reason: UnresolvedReason::NonLiteral,
            };
        }
        let (name, subpath) = split_package(specifier);
        if NODE_BUILTINS.contains(&name.as_str()) {
            return Resolution::Builtin { name };
        }
        Resolution::External {
            ecosystem: "npm".to_owned(),
            name,
            subpath,
            version_range: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn resolver() -> FileSetResolver {
        let paths: Vec<RepoPath> = ["src/a.ts", "src/lib/index.ts", "src/b.tsx", "x.ts"]
            .iter()
            .map(|p| RepoPath::new(*p).unwrap())
            .collect();
        FileSetResolver::new(paths.iter())
    }

    fn file(r: &Resolution) -> Option<&str> {
        match r {
            Resolution::File { path, .. } => Some(path.as_str()),
            _ => None,
        }
    }

    #[test]
    fn resolves_relative_files_extensions_and_index() {
        let r = resolver();
        let from = RepoPath::new("src/main.ts").unwrap();
        assert_eq!(
            file(&r.resolve(&from, "./a", ResolveKind::Import)),
            Some("src/a.ts")
        );
        assert_eq!(
            file(&r.resolve(&from, "./a.js", ResolveKind::Import)),
            Some("src/a.ts")
        );
        assert_eq!(
            file(&r.resolve(&from, "./lib", ResolveKind::Import)),
            Some("src/lib/index.ts")
        );
        assert_eq!(
            file(&r.resolve(&from, "../x", ResolveKind::Import)),
            Some("x.ts")
        );
    }

    #[test]
    fn missing_and_escaping_specifiers_are_unresolved() {
        let r = resolver();
        let from = RepoPath::new("src/main.ts").unwrap();
        assert!(matches!(
            r.resolve(&from, "./nope", ResolveKind::Import),
            Resolution::Unresolved {
                reason: UnresolvedReason::NotFound
            }
        ));
        assert!(matches!(
            r.resolve(&from, "../../out", ResolveKind::Import),
            Resolution::Unresolved {
                reason: UnresolvedReason::OutsideRepository
            }
        ));
    }

    #[test]
    fn bare_specifiers_are_packages_or_builtins() {
        let r = resolver();
        let from = RepoPath::new("src/main.ts").unwrap();
        match r.resolve(&from, "@nestjs/common/decorators", ResolveKind::Import) {
            Resolution::External { name, subpath, .. } => {
                assert_eq!(name, "@nestjs/common");
                assert_eq!(subpath.as_deref(), Some("decorators"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            r.resolve(&from, "node:fs", ResolveKind::Import),
            Resolution::Builtin { .. }
        ));
        assert!(matches!(
            r.resolve(&from, "path", ResolveKind::Import),
            Resolution::Builtin { .. }
        ));
    }
}
