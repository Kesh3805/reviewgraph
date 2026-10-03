//! Monorepo / workspace detection (INIT-005).
//!
//! Patterns are compiled with `globset` and matched against `FileInventory::dirs`; the file
//! system is not re-read. Only workspace definitions at the repository root are interpreted;
//! nested roots are reported as warnings.

use std::collections::{BTreeMap, BTreeSet};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::manifests::{Ecosystem, ManifestFact, ManifestFacts};
use crate::read::BoundedReader;
use crate::walk::FileInventory;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTool {
    Pnpm,
    NpmWorkspaces,
    Yarn,
    Lerna,
    Nx,
    Turbo,
    Rush,
    Cargo,
    GoWork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspacePackage {
    pub name: String,
    pub dir: RepoDir,
    pub manifest: Option<RepoPath>,
    pub ecosystem: Ecosystem,
    pub version: Option<String>,
    pub private: bool,
    pub source: WorkspaceTool,
    pub internal_deps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceLayout {
    pub is_monorepo: bool,
    pub tools: Vec<WorkspaceTool>,
    pub root_package: Option<RepoPath>,
    /// Sorted by directory.
    pub packages: Vec<WorkspacePackage>,
    /// `(from, to)` package names, sorted.
    pub internal_edges: Vec<(String, String)>,
    pub cycles: Vec<Vec<String>>,
    /// Task names declared in turbo.json.
    pub turbo_tasks: Vec<String>,
    /// nx.json `workspaceLayout.appsDir` / `libsDir`.
    pub nx_apps_dir: Option<String>,
    pub nx_libs_dir: Option<String>,
}

impl WorkspaceLayout {
    /// The package whose directory is the longest prefix of `path`.
    pub fn package_for(&self, path: &RepoPath) -> Option<&WorkspacePackage> {
        self.packages
            .iter()
            .filter(|p| !p.dir.is_root() && p.dir.contains(path.as_str()))
            .max_by_key(|p| p.dir.depth())
    }

    pub fn package_named(&self, name: &str) -> Option<&WorkspacePackage> {
        self.packages.iter().find(|p| p.name == name)
    }
}

struct Patterns {
    include: GlobSet,
    exclude: GlobSet,
    has_double_star: bool,
}

fn compile(patterns: &[String], warnings: &mut Vec<InitWarning>) -> Patterns {
    let mut include = GlobSetBuilder::new();
    let mut exclude = GlobSetBuilder::new();
    let mut has_double_star = false;
    for raw in patterns {
        let (negated, body) = match raw.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, raw.as_str()),
        };
        let body = body.trim_start_matches("./").trim_end_matches('/');
        if body.contains("**") && !negated {
            has_double_star = true;
        }
        match GlobBuilder::new(body).literal_separator(true).build() {
            Ok(glob) => {
                if negated {
                    exclude.add(glob);
                } else {
                    include.add(glob);
                }
            }
            Err(_) => warnings.push(InitWarning::new(
                "workspace_config_parse",
                None,
                format!("workspace pattern `{raw}` is not a valid glob"),
            )),
        }
    }
    Patterns {
        include: include.build().unwrap_or_else(|_| GlobSet::empty()),
        exclude: exclude.build().unwrap_or_else(|_| GlobSet::empty()),
        has_double_star,
    }
}

/// Directories of the inventory matched by `patterns`, sorted.
fn expand(
    inv: &FileInventory,
    patterns: &[String],
    warnings: &mut Vec<InitWarning>,
) -> (Vec<RepoDir>, bool) {
    let compiled = compile(patterns, warnings);
    let dirs = inv
        .dirs
        .iter()
        .filter(|d| compiled.include.is_match(d.as_str()) && !compiled.exclude.is_match(d.as_str()))
        .map(|d| RepoDir::from(d.clone()))
        .collect();
    (dirs, compiled.has_double_star)
}

fn read_text(inv: &FileInventory, reader: &BoundedReader, path: &str) -> Option<String> {
    let entry = inv.find(path)?;
    reader.read_text(entry, 512 * 1024).ok()
}

fn package_from_manifest(
    dir: &RepoDir,
    manifest: Option<&ManifestFact>,
    ecosystem: Ecosystem,
    source: WorkspaceTool,
    warnings: &mut Vec<InitWarning>,
) -> WorkspacePackage {
    let name = match manifest.and_then(|m| m.name.clone()) {
        Some(n) => n,
        None => {
            warnings.push(InitWarning::new(
                "workspace_package_unnamed",
                manifest.map(|m| m.path.clone()),
                format!("package in `{dir}` has no name; using the directory name"),
            ));
            dir.basename().to_owned()
        }
    };
    WorkspacePackage {
        name,
        dir: dir.clone(),
        manifest: manifest.map(|m| m.path.clone()),
        ecosystem,
        version: manifest.and_then(|m| m.version.clone()),
        private: manifest.map(|m| m.private).unwrap_or(false),
        source,
        internal_deps: Vec::new(),
    }
}

fn manifest_in<'a>(
    manifests: &'a ManifestFacts,
    dir: &RepoDir,
    ecosystem: Ecosystem,
) -> Option<&'a ManifestFact> {
    manifests
        .manifests
        .iter()
        .find(|m| m.ecosystem == ecosystem && m.dir() == *dir)
}

/// Detects workspace packages, internal dependency edges and cycles.
pub fn detect_workspaces(
    inv: &FileInventory,
    reader: &BoundedReader,
    manifests: &ManifestFacts,
) -> (WorkspaceLayout, Vec<InitWarning>) {
    let span = tracing::info_span!(
        "init.workspaces",
        workspace.tools = tracing::field::Empty,
        workspace.packages = tracing::field::Empty,
        workspace.cycles = tracing::field::Empty
    );
    let _guard = span.enter();

    let mut warnings = Vec::new();
    let mut layout = WorkspaceLayout::default();
    let mut tools: BTreeSet<WorkspaceTool> = BTreeSet::new();
    let mut packages: Vec<WorkspacePackage> = Vec::new();
    let root = RepoDir::root();

    layout.root_package = inv.find("package.json").map(|e| e.path.clone());

    // Nested workspace roots are not modelled.
    for entry in &inv.entries {
        let name = entry.path.as_str().rsplit('/').next().unwrap_or("");
        if name == "pnpm-workspace.yaml" && entry.path.as_str().contains('/') {
            warnings.push(InitWarning::new(
                "nested_workspace_root",
                Some(entry.path.clone()),
                "workspace definitions below the repository root are not modelled",
            ));
        }
    }

    // npm-family patterns: pnpm first, then package.json workspaces, then lerna.
    let mut npm_patterns: Vec<(WorkspaceTool, Vec<String>)> = Vec::new();
    if let Some(text) = read_text(inv, reader, "pnpm-workspace.yaml") {
        match serde_yaml::from_str::<serde_yaml::Value>(&text) {
            Ok(value) => {
                let list: Vec<String> = value
                    .get("packages")
                    .and_then(|v| v.as_sequence())
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                npm_patterns.push((WorkspaceTool::Pnpm, list));
            }
            Err(_) => warnings.push(InitWarning::new(
                "workspace_config_parse",
                RepoPath::new("pnpm-workspace.yaml").ok(),
                "pnpm-workspace.yaml could not be parsed",
            )),
        }
    }
    if npm_patterns.is_empty() {
        if let Some(root_manifest) = manifest_in(manifests, &root, Ecosystem::Npm) {
            if let Some(ws) = root_manifest
                .npm
                .as_ref()
                .and_then(|n| n.workspaces.clone())
            {
                let yarn = inv.find("yarn.lock").is_some();
                npm_patterns.push((
                    if yarn {
                        WorkspaceTool::Yarn
                    } else {
                        WorkspaceTool::NpmWorkspaces
                    },
                    ws,
                ));
            }
        }
    }
    if let Some(text) = read_text(inv, reader, "lerna.json") {
        if let Ok(value) = crate::jsonc::parse_jsonc(text.as_bytes()) {
            tools.insert(WorkspaceTool::Lerna);
            if npm_patterns.is_empty() {
                let list: Vec<String> = value
                    .get("packages")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["packages/*".to_owned()]);
                npm_patterns.push((WorkspaceTool::Lerna, list));
            }
        }
    }
    for (tool, patterns) in &npm_patterns {
        tools.insert(*tool);
        let (dirs, has_double_star) = expand(inv, patterns, &mut warnings);
        for dir in dirs {
            match manifest_in(manifests, &dir, Ecosystem::Npm) {
                Some(manifest) => packages.push(package_from_manifest(
                    &dir,
                    Some(manifest),
                    Ecosystem::Npm,
                    *tool,
                    &mut warnings,
                )),
                None if !has_double_star => warnings.push(InitWarning::new(
                    "workspace_dir_without_manifest",
                    RepoPath::new(dir.as_str()).ok(),
                    format!("`{dir}` matches a workspace pattern but has no package.json"),
                )),
                None => {}
            }
        }
    }

    // Rush.
    if let Some(text) = read_text(inv, reader, "rush.json") {
        tools.insert(WorkspaceTool::Rush);
        if let Ok(value) = crate::jsonc::parse_jsonc(text.as_bytes()) {
            for folder in value
                .get("projects")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|p| p.get("projectFolder").and_then(|f| f.as_str()))
            {
                if let Ok(dir) = RepoDir::new(folder.trim_matches('/')) {
                    if let Some(m) = manifest_in(manifests, &dir, Ecosystem::Npm) {
                        packages.push(package_from_manifest(
                            &dir,
                            Some(m),
                            Ecosystem::Npm,
                            WorkspaceTool::Rush,
                            &mut warnings,
                        ));
                    }
                }
            }
        }
    }

    // Nx: every project.json directory is a package.
    if let Some(text) = read_text(inv, reader, "nx.json") {
        tools.insert(WorkspaceTool::Nx);
        if let Ok(value) = crate::jsonc::parse_jsonc(text.as_bytes()) {
            let layout_obj = value.get("workspaceLayout");
            layout.nx_apps_dir = layout_obj
                .and_then(|l| l.get("appsDir"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            layout.nx_libs_dir = layout_obj
                .and_then(|l| l.get("libsDir"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
        }
    }
    if tools.contains(&WorkspaceTool::Nx) {
        for entry in &inv.entries {
            if entry.path.as_str().rsplit('/').next() != Some("project.json") {
                continue;
            }
            let dir = RepoDir::parent_of(&entry.path);
            if dir.is_root() || packages.iter().any(|p| p.dir == dir) {
                continue;
            }
            let name = read_text(inv, reader, entry.path.as_str())
                .and_then(|t| crate::jsonc::parse_jsonc(t.as_bytes()).ok())
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_owned));
            let manifest = manifest_in(manifests, &dir, Ecosystem::Npm);
            let mut pkg = package_from_manifest(
                &dir,
                manifest,
                Ecosystem::Npm,
                WorkspaceTool::Nx,
                &mut warnings,
            );
            if let Some(name) = name {
                pkg.name = name;
            }
            pkg.manifest = manifest
                .map(|m| m.path.clone())
                .or(Some(entry.path.clone()));
            packages.push(pkg);
        }
    }

    // Turbo: no packages, only the tool and its task names.
    if let Some(text) = read_text(inv, reader, "turbo.json") {
        tools.insert(WorkspaceTool::Turbo);
        if let Ok(value) = crate::jsonc::parse_jsonc(text.as_bytes()) {
            for key in ["tasks", "pipeline"] {
                if let Some(map) = value.get(key).and_then(|v| v.as_object()) {
                    layout.turbo_tasks.extend(map.keys().cloned());
                }
            }
            layout.turbo_tasks.sort();
            layout.turbo_tasks.dedup();
        }
    }

    // Cargo workspace.
    if let Some(cargo_root) = manifest_in(manifests, &root, Ecosystem::Cargo) {
        if cargo_root.meta.contains_key("workspace") {
            tools.insert(WorkspaceTool::Cargo);
            let mut patterns = cargo_root.members.clone();
            if let Some(text) = read_text(inv, reader, "Cargo.toml") {
                if let Ok(table) = text.parse::<toml::Table>() {
                    if let Some(exclude) = table
                        .get("workspace")
                        .and_then(|w| w.get("exclude"))
                        .and_then(|e| e.as_array())
                    {
                        patterns.extend(
                            exclude
                                .iter()
                                .filter_map(|v| v.as_str())
                                .map(|e| format!("!{e}")),
                        );
                    }
                }
            }
            let (dirs, _) = expand(inv, &patterns, &mut warnings);
            for dir in dirs {
                if let Some(m) = manifest_in(manifests, &dir, Ecosystem::Cargo) {
                    packages.push(package_from_manifest(
                        &dir,
                        Some(m),
                        Ecosystem::Cargo,
                        WorkspaceTool::Cargo,
                        &mut warnings,
                    ));
                }
            }
        }
    }

    // go.work
    if let Some(text) = read_text(inv, reader, "go.work") {
        tools.insert(WorkspaceTool::GoWork);
        let mut in_use = false;
        let mut uses = Vec::new();
        for raw in text.lines() {
            let line = raw.split("//").next().unwrap_or(raw).trim();
            if in_use {
                if line == ")" {
                    in_use = false;
                } else if !line.is_empty() {
                    uses.push(line.to_owned());
                }
            } else if let Some(rest) = line.strip_prefix("use") {
                let rest = rest.trim();
                if rest == "(" {
                    in_use = true;
                } else if !rest.is_empty() {
                    uses.push(rest.to_owned());
                }
            }
        }
        for used in uses {
            let used = used
                .trim_matches('"')
                .trim_start_matches("./")
                .trim_end_matches('/');
            let dir = if used == "." {
                RepoDir::root()
            } else if let Ok(d) = RepoDir::new(used) {
                d
            } else {
                continue;
            };
            if let Some(m) = manifest_in(manifests, &dir, Ecosystem::Go) {
                packages.push(package_from_manifest(
                    &dir,
                    Some(m),
                    Ecosystem::Go,
                    WorkspaceTool::GoWork,
                    &mut warnings,
                ));
            }
        }
    }

    // Order, deduplicate by directory, warn about duplicate names.
    packages.sort_by(|a, b| a.dir.cmp(&b.dir));
    packages.dedup_by(|b, a| a.dir == b.dir);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for pkg in &packages {
        if !seen.insert(pkg.name.clone()) {
            warnings.push(InitWarning::new(
                "duplicate_workspace_package_name",
                pkg.manifest.clone(),
                format!(
                    "workspace package name `{}` is declared more than once",
                    pkg.name
                ),
            ));
        }
    }

    // Internal edges.
    let by_name: BTreeMap<&str, usize> = {
        let mut map = BTreeMap::new();
        for (i, p) in packages.iter().enumerate() {
            map.entry(p.name.as_str()).or_insert(i);
        }
        map
    };
    let mut edges: BTreeSet<(String, String)> = BTreeSet::new();
    let mut internal: Vec<Vec<String>> = vec![Vec::new(); packages.len()];
    for (i, pkg) in packages.iter().enumerate() {
        let Some(manifest) = pkg
            .manifest
            .as_ref()
            .and_then(|path| manifests.manifest(path.as_str()))
        else {
            continue;
        };
        for dep in manifest.dependencies.keys() {
            if let Some(&j) = by_name.get(dep.as_str()) {
                if j != i && packages[j].ecosystem == pkg.ecosystem {
                    edges.insert((pkg.name.clone(), dep.clone()));
                    internal[i].push(dep.clone());
                }
            }
        }
    }
    for (pkg, deps) in packages.iter_mut().zip(internal) {
        pkg.internal_deps = deps;
        pkg.internal_deps.sort();
    }
    layout.internal_edges = edges.into_iter().collect();
    layout.cycles = cycles(&packages, &layout.internal_edges);
    for cycle in &layout.cycles {
        warnings.push(InitWarning::new(
            "workspace_cycle",
            None,
            format!(
                "workspace packages depend on each other: {}",
                cycle.join(" -> ")
            ),
        ));
    }

    layout.is_monorepo = packages.len() >= 2;
    layout.tools = tools.into_iter().collect();
    layout.packages = packages;
    warnings.sort();
    span.record("workspace.tools", layout.tools.len());
    span.record("workspace.packages", layout.packages.len());
    span.record("workspace.cycles", layout.cycles.len());
    (layout, warnings)
}

/// Strongly connected components with more than one member (Tarjan).
fn cycles(packages: &[WorkspacePackage], edges: &[(String, String)]) -> Vec<Vec<String>> {
    let index_of: BTreeMap<&str, usize> = packages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.as_str(), i))
        .collect();
    let n = packages.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, to) in edges {
        if let (Some(&a), Some(&b)) = (index_of.get(from.as_str()), index_of.get(to.as_str())) {
            adj[a].push(b);
        }
    }
    struct State {
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(v: usize, adj: &[Vec<usize>], s: &mut State) {
        s.index[v] = Some(s.next);
        s.low[v] = s.next;
        s.next += 1;
        s.stack.push(v);
        s.on_stack[v] = true;
        for &w in &adj[v] {
            match s.index[w] {
                None => {
                    visit(w, adj, s);
                    s.low[v] = s.low[v].min(s.low[w]);
                }
                Some(wi) if s.on_stack[w] => s.low[v] = s.low[v].min(wi),
                Some(_) => {}
            }
        }
        if Some(s.low[v]) == s.index[v] {
            let mut component = Vec::new();
            while let Some(w) = s.stack.pop() {
                s.on_stack[w] = false;
                component.push(w);
                if w == v {
                    break;
                }
            }
            if component.len() > 1 {
                s.out.push(component);
            }
        }
    }
    let mut state = State {
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in 0..n {
        if state.index[v].is_none() {
            visit(v, &adj, &mut state);
        }
    }
    let mut result: Vec<Vec<String>> = state
        .out
        .into_iter()
        .map(|c| {
            let mut names: Vec<String> = c.into_iter().map(|i| packages[i].name.clone()).collect();
            names.sort();
            names
        })
        .collect();
    result.sort();
    result
}
