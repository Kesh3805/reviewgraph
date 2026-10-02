//! Workspace maintenance checks. The dependency-direction rules from
//! docs/architecture/target-architecture.md §2.1 live here so they are executable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Internal crates each library crate may depend on. Apps and `pipeline` are composition
/// roots and may depend on any library crate; no crate may depend on an app.
pub const ALLOWED: &[(&str, &[&str])] = &[
    ("review-core", &[]),
    ("telemetry", &["review-core"]),
    ("repository", &["review-core", "telemetry"]),
    ("analysis-ir", &["review-core"]),
    (
        "lang-typescript",
        &["review-core", "analysis-ir", "telemetry"],
    ),
    ("codegraph", &["review-core", "analysis-ir", "telemetry"]),
    (
        "graph-storage",
        &["review-core", "analysis-ir", "codegraph", "telemetry"],
    ),
    (
        "incremental",
        &[
            "review-core",
            "analysis-ir",
            "codegraph",
            "graph-storage",
            "repository",
            "telemetry",
        ],
    ),
    (
        "diff-engine",
        &[
            "review-core",
            "repository",
            "analysis-ir",
            "codegraph",
            "telemetry",
        ],
    ),
    (
        "impact",
        &[
            "review-core",
            "analysis-ir",
            "codegraph",
            "diff-engine",
            "telemetry",
        ],
    ),
    ("semantic", &["review-core", "codegraph", "telemetry"]),
    (
        "profile",
        &[
            "review-core",
            "repository",
            "analysis-ir",
            "codegraph",
            "telemetry",
        ],
    ),
    (
        "context-engine",
        &[
            "review-core",
            "analysis-ir",
            "codegraph",
            "diff-engine",
            "impact",
            "semantic",
            "profile",
            "telemetry",
        ],
    ),
    ("model-gateway", &["review-core", "telemetry"]),
    (
        "reviewers",
        &[
            "review-core",
            "codegraph",
            "diff-engine",
            "impact",
            "context-engine",
            "profile",
            "model-gateway",
            "telemetry",
        ],
    ),
    (
        "verification",
        &[
            "review-core",
            "analysis-ir",
            "codegraph",
            "diff-engine",
            "impact",
            "profile",
            "model-gateway",
            "telemetry",
        ],
    ),
];

/// External crates that specific internal crates must never depend on directly.
/// Reviewers and verification reach models only through the `model-gateway` trait (ADR-009);
/// the domain crates stay free of I/O.
pub const BANNED: &[(&str, &[&str])] = &[
    ("reviewers", &["reqwest", "hyper", "sqlx"]),
    ("verification", &["reqwest", "hyper", "sqlx"]),
    ("review-core", &["reqwest", "sqlx", "tokio", "tree-sitter"]),
    ("analysis-ir", &["reqwest", "sqlx", "tokio", "tree-sitter"]),
];

/// Library crates allowed to depend on any other library crate.
pub const COMPOSITION_ROOTS: &[&str] = &["pipeline"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub krate: String,
    pub dependency: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub dir: PathBuf,
    pub is_app: bool,
    /// Normal and build dependencies; dev-dependencies are excluded.
    pub deps: BTreeSet<String>,
}

/// Reads every `crates/*/Cargo.toml` and `apps/*/Cargo.toml` under `engine_dir`.
pub fn read_workspace(engine_dir: &Path) -> anyhow::Result<BTreeMap<String, CrateInfo>> {
    let mut out = BTreeMap::new();
    for (group, is_app) in [("crates", false), ("apps", true)] {
        let dir = engine_dir.join(group);
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            let manifest = path.join("Cargo.toml");
            if !manifest.exists() {
                continue;
            }
            let doc: toml::Table = std::fs::read_to_string(&manifest)?.parse()?;
            let name = doc
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| anyhow::anyhow!("{} has no package.name", manifest.display()))?
                .to_string();
            let mut deps = BTreeSet::new();
            for section in ["dependencies", "build-dependencies"] {
                if let Some(t) = doc.get(section).and_then(|d| d.as_table()) {
                    deps.extend(t.keys().cloned());
                }
            }
            out.insert(
                name,
                CrateInfo {
                    dir: path,
                    is_app,
                    deps,
                },
            );
        }
    }
    Ok(out)
}

/// Returns every violation of the dependency rules. Empty means the workspace is compliant.
pub fn check(engine_dir: &Path) -> anyhow::Result<Vec<Violation>> {
    let ws = read_workspace(engine_dir)?;
    let allowed: BTreeMap<&str, BTreeSet<&str>> = ALLOWED
        .iter()
        .map(|(k, v)| (*k, v.iter().copied().collect()))
        .collect();
    let banned: BTreeMap<&str, &[&str]> = BANNED.iter().copied().collect();
    let mut violations = Vec::new();

    for (name, info) in &ws {
        let is_root = info.is_app || COMPOSITION_ROOTS.contains(&name.as_str());
        if !is_root && !allowed.contains_key(name.as_str()) {
            violations.push(Violation {
                krate: name.clone(),
                dependency: String::new(),
                reason: "library crate missing from the ALLOWED table",
            });
            continue;
        }
        for dep in &info.deps {
            if let Some(target) = ws.get(dep) {
                if target.is_app {
                    violations.push(Violation {
                        krate: name.clone(),
                        dependency: dep.clone(),
                        reason: "no crate may depend on an app",
                    });
                    continue;
                }
                let ok = is_root
                    || allowed
                        .get(name.as_str())
                        .is_some_and(|a| a.contains(dep.as_str()));
                if !ok {
                    violations.push(Violation {
                        krate: name.clone(),
                        dependency: dep.clone(),
                        reason: "internal dependency not allowed by target-architecture §2.1",
                    });
                }
            }
            if !info.is_app && dep == "anyhow" {
                violations.push(Violation {
                    krate: name.clone(),
                    dependency: dep.clone(),
                    reason:
                        "library crates must use typed errors; anyhow is for apps only (DOM-002)",
                });
            }
            if banned
                .get(name.as_str())
                .is_some_and(|b| b.contains(&dep.as_str()))
            {
                violations.push(Violation {
                    krate: name.clone(),
                    dependency: dep.clone(),
                    reason: "banned external dependency",
                });
            }
        }
    }
    Ok(violations)
}
