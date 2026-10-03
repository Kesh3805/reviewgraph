//! Recorded-only ecosystems: Gemfile, composer.json, *.csproj.

use std::sync::OnceLock;

use regex::Regex;
use review_core::location::RepoPath;
use serde_json::Value;

use super::{DepKind, DepSpec, Ecosystem, ManifestFact};
use crate::error::InitWarning;

fn add(fact: &mut ManifestFact, name: &str, range: &str, kind: DepKind) {
    fact.dependencies.entry(name.to_owned()).or_insert(DepSpec {
        range: range.to_owned(),
        kind,
        workspace_protocol: false,
    });
}

pub(crate) fn parse_gemfile(path: &RepoPath, text: &str) -> (ManifestFact, Vec<InitWarning>) {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"^\s*gem\s+['"]([^'"]+)['"](?:\s*,\s*['"]([^'"]+)['"])?"#).ok()
    });
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Rubygems);
    if let Some(re) = re {
        for line in text.lines() {
            if let Some(c) = re.captures(line) {
                let name = c.get(1).map(|m| m.as_str()).unwrap_or_default();
                let range = c.get(2).map(|m| m.as_str()).unwrap_or("*");
                add(&mut fact, name, range, DepKind::Prod);
            }
        }
    }
    (fact, Vec::new())
}

pub(crate) fn parse_composer(path: &RepoPath, text: &str) -> (ManifestFact, Vec<InitWarning>) {
    let value: Value = match crate::jsonc::parse_jsonc(text.as_bytes()) {
        Ok(v) => v,
        Err(e) => {
            return (
                ManifestFact::failed(path.clone(), Ecosystem::Composer, "invalid JSON"),
                vec![InitWarning::new(
                    "manifest_parse",
                    Some(path.clone()),
                    format!("line {}, column {}", e.line, e.column),
                )],
            );
        }
    };
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Composer);
    fact.name = value.get("name").and_then(Value::as_str).map(str::to_owned);
    fact.version = value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    for (key, kind) in [("require", DepKind::Prod), ("require-dev", DepKind::Dev)] {
        if let Some(map) = value.get(key).and_then(Value::as_object) {
            for (name, range) in map {
                if name == "php" || name.starts_with("ext-") {
                    continue;
                }
                add(&mut fact, name, range.as_str().unwrap_or("*"), kind);
            }
        }
    }
    (fact, Vec::new())
}

pub(crate) fn parse_csproj(path: &RepoPath, text: &str) -> (ManifestFact, Vec<InitWarning>) {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"<PackageReference\s+Include="([^"]+)"(?:\s+Version="([^"]+)")?"#).ok()
    });
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Nuget);
    let file = path.as_str().rsplit('/').next().unwrap_or(path.as_str());
    fact.name = Some(file.trim_end_matches(".csproj").to_owned());
    if let Some(re) = re {
        for c in re.captures_iter(text) {
            let name = c.get(1).map(|m| m.as_str()).unwrap_or_default();
            let range = c.get(2).map(|m| m.as_str()).unwrap_or("*");
            add(&mut fact, name, range, DepKind::Prod);
        }
    }
    (fact, Vec::new())
}
