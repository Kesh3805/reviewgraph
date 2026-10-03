//! Python manifests: pyproject.toml (PEP 621 and Poetry), requirements*.txt, Pipfile.

use review_core::location::RepoPath;

use super::{DepKind, DepSpec, Ecosystem, ManifestFact};
use crate::error::InitWarning;

/// Splits a PEP 508 requirement into `(name, rest)`. The name ends at the first of
/// whitespace or `[<>=!~;@(`.
fn split_requirement(req: &str) -> Option<(String, String)> {
    let req = req.split('#').next().unwrap_or(req).trim();
    if req.is_empty() {
        return None;
    }
    let end = req
        .find(|c: char| c.is_whitespace() || "[<>=!~;@(".contains(c))
        .unwrap_or(req.len());
    let name = req[..end].trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_owned(), req[end..].trim().to_owned()))
}

fn add_dep(fact: &mut ManifestFact, req: &str, kind: DepKind) {
    if let Some((name, range)) = split_requirement(req) {
        fact.dependencies.entry(name).or_insert(DepSpec {
            range,
            kind,
            workspace_protocol: false,
        });
    }
}

fn range_of(spec: &toml::Value) -> String {
    match spec {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("*")
            .to_owned(),
        _ => "*".to_owned(),
    }
}

fn add_table(fact: &mut ManifestFact, deps: &toml::Table, kind: DepKind) {
    for (name, spec) in deps {
        if name == "python" {
            continue;
        }
        fact.dependencies.entry(name.clone()).or_insert(DepSpec {
            range: range_of(spec),
            kind,
            workspace_protocol: false,
        });
    }
}

pub(crate) fn parse(
    path: &RepoPath,
    lower_name: &str,
    text: &str,
) -> (ManifestFact, Vec<InitWarning>) {
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Pypi);
    let mut warnings = Vec::new();
    if lower_name.starts_with("requirements") {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with('-')
                || line.contains("://")
            {
                continue;
            }
            add_dep(&mut fact, line, DepKind::Prod);
        }
        return (fact, warnings);
    }

    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            warnings.push(InitWarning::new(
                "manifest_parse",
                Some(path.clone()),
                e.message().to_owned(),
            ));
            return (
                ManifestFact::failed(path.clone(), Ecosystem::Pypi, "invalid TOML"),
                warnings,
            );
        }
    };

    if lower_name == "pipfile" {
        for (section, kind) in [("packages", DepKind::Prod), ("dev-packages", DepKind::Dev)] {
            if let Some(deps) = table.get(section).and_then(|v| v.as_table()) {
                add_table(&mut fact, deps, kind);
            }
        }
        return (fact, warnings);
    }

    // pyproject.toml
    if let Some(project) = table.get("project").and_then(|v| v.as_table()) {
        fact.name = project
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        fact.version = project
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        if let Some(deps) = project.get("dependencies").and_then(|v| v.as_array()) {
            for dep in deps.iter().filter_map(|v| v.as_str()) {
                add_dep(&mut fact, dep, DepKind::Prod);
            }
        }
        if let Some(optional) = project
            .get("optional-dependencies")
            .and_then(|v| v.as_table())
        {
            for group in optional.values().filter_map(|v| v.as_array()) {
                for dep in group.iter().filter_map(|v| v.as_str()) {
                    add_dep(&mut fact, dep, DepKind::Optional);
                }
            }
        }
    }
    if let Some(poetry) = table
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|v| v.as_table())
    {
        fact.meta
            .insert("tool_poetry".to_owned(), "true".to_owned());
        if fact.name.is_none() {
            fact.name = poetry
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
        }
        if fact.version.is_none() {
            fact.version = poetry
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
        }
        if let Some(deps) = poetry.get("dependencies").and_then(|v| v.as_table()) {
            add_table(&mut fact, deps, DepKind::Prod);
        }
        if let Some(deps) = poetry.get("dev-dependencies").and_then(|v| v.as_table()) {
            add_table(&mut fact, deps, DepKind::Dev);
        }
        if let Some(groups) = poetry.get("group").and_then(|v| v.as_table()) {
            for deps in groups
                .values()
                .filter_map(|g| g.get("dependencies"))
                .filter_map(|d| d.as_table())
            {
                add_table(&mut fact, deps, DepKind::Dev);
            }
        }
    }
    for tool in ["ruff", "flake8", "pylint", "mypy", "black", "pytest"] {
        if table.get("tool").and_then(|t| t.get(tool)).is_some() {
            fact.meta.insert(format!("tool_{tool}"), "true".to_owned());
        }
    }
    (fact, warnings)
}
