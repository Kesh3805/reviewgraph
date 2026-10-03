//! Cargo.toml: package identity, workspace members and dependency names.

use review_core::location::RepoPath;

use super::{DepKind, DepSpec, Ecosystem, ManifestFact};
use crate::error::InitWarning;

fn range_of(spec: &toml::Value) -> (String, bool) {
    match spec {
        toml::Value::String(s) => (s.clone(), false),
        toml::Value::Table(t) => {
            let workspace = t
                .get("workspace")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let range = t
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if workspace {
                        "workspace".to_owned()
                    } else {
                        "*".to_owned()
                    }
                });
            (range, workspace || t.contains_key("path"))
        }
        _ => ("*".to_owned(), false),
    }
}

pub(crate) fn parse(path: &RepoPath, text: &str) -> (ManifestFact, Vec<InitWarning>) {
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            return (
                ManifestFact::failed(path.clone(), Ecosystem::Cargo, "invalid TOML"),
                vec![InitWarning::new(
                    "manifest_parse",
                    Some(path.clone()),
                    e.message().to_owned(),
                )],
            );
        }
    };
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Cargo);
    if let Some(package) = table.get("package").and_then(|v| v.as_table()) {
        fact.name = package
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        fact.version = package
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        fact.private = package.get("publish").and_then(|v| v.as_bool()) == Some(false);
    }
    if let Some(workspace) = table.get("workspace").and_then(|v| v.as_table()) {
        fact.meta.insert("workspace".to_owned(), "true".to_owned());
        if let Some(members) = workspace.get("members").and_then(|v| v.as_array()) {
            fact.members = members
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
        }
    }
    for (section, kind) in [
        ("dependencies", DepKind::Prod),
        ("dev-dependencies", DepKind::Dev),
        ("build-dependencies", DepKind::Dev),
    ] {
        if let Some(deps) = table.get(section).and_then(|v| v.as_table()) {
            for (name, spec) in deps {
                let (range, workspace_protocol) = range_of(spec);
                fact.dependencies.entry(name.clone()).or_insert(DepSpec {
                    range,
                    kind,
                    workspace_protocol,
                });
            }
        }
    }
    (fact, Vec::new())
}
