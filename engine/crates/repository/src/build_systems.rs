//! Build-system detection from config file presence (INIT-004).

use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dirs::RepoDir;
use crate::walk::FileInventory;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BuildSystemKind {
    NestCli,
    Tsc,
    Webpack,
    Vite,
    Rollup,
    Esbuild,
    Tsup,
    Swc,
    Turbo,
    Nx,
    Make,
    Bazel,
    Maven,
    Gradle,
    Cargo,
    GoBuild,
    Just,
    Taskfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildSystemFact {
    pub kind: BuildSystemKind,
    pub config: RepoPath,
    pub scope_dir: RepoDir,
}

fn kind_of(name: &str) -> Option<BuildSystemKind> {
    let lower = name.to_ascii_lowercase();
    let prefix = |p: &str| {
        lower
            .strip_prefix(p)
            .is_some_and(|rest| rest.starts_with('.') && !rest[1..].is_empty())
    };
    match lower.as_str() {
        "nest-cli.json" => return Some(BuildSystemKind::NestCli),
        "tsconfig.build.json" => return Some(BuildSystemKind::Tsc),
        ".swcrc" => return Some(BuildSystemKind::Swc),
        "turbo.json" => return Some(BuildSystemKind::Turbo),
        "nx.json" => return Some(BuildSystemKind::Nx),
        "makefile" | "gnumakefile" => return Some(BuildSystemKind::Make),
        "workspace" | "module.bazel" => return Some(BuildSystemKind::Bazel),
        "pom.xml" => return Some(BuildSystemKind::Maven),
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => {
            return Some(BuildSystemKind::Gradle)
        }
        "cargo.toml" => return Some(BuildSystemKind::Cargo),
        "go.mod" => return Some(BuildSystemKind::GoBuild),
        "justfile" => return Some(BuildSystemKind::Just),
        "taskfile.yml" | "taskfile.yaml" => return Some(BuildSystemKind::Taskfile),
        _ => {}
    }
    if prefix("webpack.config") {
        Some(BuildSystemKind::Webpack)
    } else if prefix("vite.config") {
        Some(BuildSystemKind::Vite)
    } else if prefix("rollup.config") {
        Some(BuildSystemKind::Rollup)
    } else if prefix("esbuild") {
        Some(BuildSystemKind::Esbuild)
    } else if prefix("tsup.config") {
        Some(BuildSystemKind::Tsup)
    } else {
        None
    }
}

/// Build systems implied by config files anywhere in the inventory, sorted by path.
pub fn detect_build_systems(inventory: &FileInventory) -> Vec<BuildSystemFact> {
    let mut out = Vec::new();
    for entry in &inventory.entries {
        let name = entry.path.as_str().rsplit('/').next().unwrap_or("");
        if let Some(kind) = kind_of(name) {
            out.push(BuildSystemFact {
                kind,
                config: entry.path.clone(),
                scope_dir: RepoDir::parent_of(&entry.path),
            });
        }
    }
    out.sort_by(|a, b| a.config.cmp(&b.config).then(a.kind.cmp(&b.kind)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_kinds() {
        assert_eq!(kind_of("nest-cli.json"), Some(BuildSystemKind::NestCli));
        assert_eq!(kind_of("vite.config.ts"), Some(BuildSystemKind::Vite));
        assert_eq!(kind_of("webpack.config.js"), Some(BuildSystemKind::Webpack));
        assert_eq!(kind_of("esbuild.mjs"), Some(BuildSystemKind::Esbuild));
        assert_eq!(kind_of("Makefile"), Some(BuildSystemKind::Make));
        assert_eq!(kind_of("WORKSPACE"), Some(BuildSystemKind::Bazel));
        assert_eq!(kind_of("vite.config"), None);
        assert_eq!(kind_of("README.md"), None);
    }
}
