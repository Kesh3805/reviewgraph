//! Lint/format, compiler/runtime, CI and git-hook configuration (INIT-007).
//!
//! Config files written as code are never evaluated. CI `run:` commands are reduced to their
//! first token because they can embed inline secrets; `${{ secrets.X }}` names are not kept.

use std::collections::BTreeSet;

use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::InitWarning;
use crate::manifests::{Ecosystem, ManifestFacts};
use crate::read::BoundedReader;
use crate::tsconfig::TsConfigSet;
use crate::walk::FileInventory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolFact {
    pub tool: String,
    pub path: RepoPath,
    /// A short identifier-like detail (builder name, runtime version), never free text.
    pub detail: Option<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CiProvider {
    GithubActions,
    Gitlab,
    CircleCi,
    Jenkins,
    AzurePipelines,
    Bitbucket,
    Buildkite,
    CloudBuild,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct CiWorkflow {
    pub name: Option<String>,
    pub triggers: Vec<String>,
    pub jobs: Vec<String>,
    /// `uses:` action references such as `actions/checkout@v4`.
    pub actions: Vec<String>,
    /// First tokens of `run:` commands only (`npm`, `docker`, ...).
    pub run_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CiFact {
    pub provider: CiProvider,
    pub path: RepoPath,
    pub workflow: Option<CiWorkflow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ToolingFacts {
    pub lint: Vec<ToolFact>,
    pub compiler: Vec<ToolFact>,
    pub ci: Vec<CiFact>,
    pub hooks: Vec<ToolFact>,
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn lint_tool(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with(".eslintrc")
        || matches!(
            lower.as_str(),
            "eslint.config.js" | "eslint.config.mjs" | "eslint.config.cjs" | "eslint.config.ts"
        )
    {
        return Some("eslint");
    }
    if lower == "biome.json" || lower == "biome.jsonc" {
        return Some("biome");
    }
    if lower.starts_with(".prettierrc") || lower.starts_with("prettier.config.") {
        return Some("prettier");
    }
    if lower.starts_with(".stylelintrc") {
        return Some("stylelint");
    }
    match lower.as_str() {
        ".editorconfig" => Some("editorconfig"),
        "tslint.json" => Some("tslint"),
        ".dependency-cruiser.js" | ".dependency-cruiser.cjs" => Some("dependency-cruiser"),
        "knip.json" => Some("knip"),
        ".jscpd.json" => Some("jscpd"),
        _ => None,
    }
}

fn compiler_tool(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower == ".swcrc" {
        return Some("swc");
    }
    if lower.starts_with("babel.config.") || lower == ".babelrc" {
        return Some("babel");
    }
    match lower.as_str() {
        "nest-cli.json" => Some("nest-cli"),
        ".nvmrc" | ".node-version" => Some("node"),
        ".python-version" => Some("python"),
        "rust-toolchain" | "rust-toolchain.toml" => Some("rust"),
        _ => None,
    }
}

fn ci_provider(path: &str) -> Option<CiProvider> {
    let name = basename(path);
    if path.starts_with(".github/workflows/") && (name.ends_with(".yml") || name.ends_with(".yaml"))
    {
        return Some(CiProvider::GithubActions);
    }
    match name {
        ".gitlab-ci.yml" => Some(CiProvider::Gitlab),
        "Jenkinsfile" => Some(CiProvider::Jenkins),
        "azure-pipelines.yml" => Some(CiProvider::AzurePipelines),
        "bitbucket-pipelines.yml" => Some(CiProvider::Bitbucket),
        "cloudbuild.yaml" | "cloudbuild.yml" => Some(CiProvider::CloudBuild),
        "config.yml" if path == ".circleci/config.yml" => Some(CiProvider::CircleCi),
        _ if path.starts_with(".buildkite/") => Some(CiProvider::Buildkite),
        _ => None,
    }
}

fn hook_tool(path: &str) -> Option<&'static str> {
    let name = basename(path);
    if path.starts_with(".husky/") && !path.starts_with(".husky/_/") {
        return Some("husky");
    }
    if name.starts_with("lint-staged.config.") || name == ".lintstagedrc" {
        return Some("lint-staged");
    }
    match name {
        ".pre-commit-config.yaml" => Some("pre-commit"),
        "lefthook.yml" | "lefthook.yaml" => Some("lefthook"),
        _ => None,
    }
}

fn first_token(line: &str) -> Option<String> {
    let token = line
        .split_whitespace()
        .find(|t| !t.contains('=') && !t.starts_with('$') && !t.starts_with('-'))?;
    let token = token.trim_matches(|c: char| !(c.is_alphanumeric() || "_-./".contains(c)));
    if token.is_empty() || token.len() > 40 {
        return None;
    }
    Some(token.to_owned())
}

fn parse_workflow(text: &str) -> Option<CiWorkflow> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    let map = value.as_mapping()?;
    let get = |key: &str| {
        map.get(serde_yaml::Value::String(key.to_owned()))
            // YAML 1.1 loaders read `on` as the boolean key `true`.
            .or_else(|| {
                if key == "on" {
                    map.get(serde_yaml::Value::Bool(true))
                } else {
                    None
                }
            })
    };
    let mut wf = CiWorkflow {
        name: get("name").and_then(|v| v.as_str()).map(str::to_owned),
        ..CiWorkflow::default()
    };
    match get("on") {
        Some(serde_yaml::Value::String(s)) => wf.triggers.push(s.clone()),
        Some(serde_yaml::Value::Sequence(seq)) => {
            wf.triggers
                .extend(seq.iter().filter_map(|v| v.as_str().map(str::to_owned)));
        }
        Some(serde_yaml::Value::Mapping(m)) => {
            wf.triggers
                .extend(m.keys().filter_map(|k| k.as_str().map(str::to_owned)));
        }
        _ => {}
    }
    let mut actions = BTreeSet::new();
    let mut tools = BTreeSet::new();
    if let Some(jobs) = get("jobs").and_then(|v| v.as_mapping()) {
        for (id, job) in jobs {
            if let Some(id) = id.as_str() {
                wf.jobs.push(id.to_owned());
            }
            if let Some(uses) = job.get("uses").and_then(|v| v.as_str()) {
                actions.insert(uses.to_owned());
            }
            for step in job
                .get("steps")
                .and_then(|s| s.as_sequence())
                .into_iter()
                .flatten()
            {
                if let Some(uses) = step.get("uses").and_then(|v| v.as_str()) {
                    actions.insert(uses.to_owned());
                }
                if let Some(run) = step.get("run").and_then(|v| v.as_str()) {
                    for line in run.lines() {
                        if let Some(tok) = first_token(line) {
                            tools.insert(tok);
                        }
                    }
                }
            }
        }
    }
    wf.triggers.sort();
    wf.jobs.sort();
    wf.actions = actions.into_iter().collect();
    wf.run_tools = tools.into_iter().collect();
    Some(wf)
}

/// Detects lint/format, compiler, CI and hook configuration.
pub fn detect_tooling(
    inv: &FileInventory,
    reader: &BoundedReader,
    tsconfigs: &TsConfigSet,
    manifests: &ManifestFacts,
) -> (ToolingFacts, Vec<InitWarning>) {
    let mut warnings = Vec::new();
    let mut facts = ToolingFacts::default();

    for entry in &inv.entries {
        let path = entry.path.as_str();
        let name = basename(path);
        if let Some(tool) = lint_tool(name) {
            facts.lint.push(ToolFact {
                tool: tool.to_owned(),
                path: entry.path.clone(),
                detail: None,
            });
        }
        if let Some(tool) = compiler_tool(name) {
            let mut detail = None;
            if tool == "node" || tool == "python" {
                detail = reader
                    .read_text(entry, 64)
                    .ok()
                    .and_then(|t| t.lines().next().map(|l| l.trim().to_owned()))
                    .filter(|v| {
                        !v.is_empty()
                            && v.len() <= 24
                            && v.chars()
                                .all(|c| c.is_ascii_alphanumeric() || "._-/*".contains(c))
                    });
            } else if tool == "nest-cli" {
                detail = reader
                    .read_text(entry, 64 * 1024)
                    .ok()
                    .and_then(|t| crate::jsonc::parse_jsonc(t.as_bytes()).ok())
                    .and_then(|v| {
                        v.get("compilerOptions")
                            .and_then(|c| c.get("builder"))
                            .and_then(|b| b.as_str().map(str::to_owned))
                    });
            }
            facts.compiler.push(ToolFact {
                tool: tool.to_owned(),
                path: entry.path.clone(),
                detail,
            });
        }
        if let Some(provider) = ci_provider(path) {
            let mut workflow = None;
            if provider == CiProvider::GithubActions {
                match reader.read_text(entry, 256 * 1024) {
                    Ok(text) => match parse_workflow(&text) {
                        Some(wf) => workflow = Some(wf),
                        None => warnings.push(InitWarning::new(
                            "ci_config_parse",
                            Some(entry.path.clone()),
                            "workflow is not valid YAML",
                        )),
                    },
                    Err(_) => warnings.push(InitWarning::new(
                        "ci_config_parse",
                        Some(entry.path.clone()),
                        "workflow could not be read",
                    )),
                }
            }
            facts.ci.push(CiFact {
                provider,
                path: entry.path.clone(),
                workflow,
            });
        }
        if let Some(tool) = hook_tool(path) {
            facts.hooks.push(ToolFact {
                tool: tool.to_owned(),
                path: entry.path.clone(),
                detail: None,
            });
        }
    }

    for config in &tsconfigs.configs {
        facts.compiler.push(ToolFact {
            tool: "tsconfig".to_owned(),
            path: config.path.clone(),
            detail: None,
        });
    }
    for manifest in &manifests.manifests {
        match manifest.ecosystem {
            Ecosystem::Npm => {
                if let Some(node) = manifest
                    .npm
                    .as_ref()
                    .and_then(|n| n.engines.get("node"))
                    .filter(|v| v.len() <= 24)
                {
                    facts.compiler.push(ToolFact {
                        tool: "node".to_owned(),
                        path: manifest.path.clone(),
                        detail: Some(node.clone()),
                    });
                }
            }
            Ecosystem::Pypi => {
                for tool in ["ruff", "flake8", "pylint", "mypy"] {
                    if manifest.meta.contains_key(&format!("tool_{tool}")) {
                        facts.lint.push(ToolFact {
                            tool: tool.to_owned(),
                            path: manifest.path.clone(),
                            detail: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    facts
        .lint
        .sort_by(|a, b| (&a.path, &a.tool).cmp(&(&b.path, &b.tool)));
    facts
        .compiler
        .sort_by(|a, b| (&a.path, &a.tool).cmp(&(&b.path, &b.tool)));
    facts
        .hooks
        .sort_by(|a, b| (&a.path, &a.tool).cmp(&(&b.path, &b.tool)));
    facts
        .ci
        .sort_by(|a, b| (&a.path, a.provider).cmp(&(&b.path, b.provider)));
    warnings.sort();
    (facts, warnings)
}
