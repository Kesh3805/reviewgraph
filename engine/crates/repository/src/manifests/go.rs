//! go.mod: module path, go directive and requirements.

use review_core::location::RepoPath;

use super::{DepKind, DepSpec, Ecosystem, ManifestFact};
use crate::error::InitWarning;

pub(crate) fn parse(path: &RepoPath, text: &str) -> (ManifestFact, Vec<InitWarning>) {
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Go);
    let mut in_require = false;
    let mut count = 0u32;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or(raw).trim();
        if line.is_empty() {
            continue;
        }
        if in_require {
            if line == ")" {
                in_require = false;
                continue;
            }
            if let Some((name, version)) = line.split_once(char::is_whitespace) {
                add(&mut fact, name, version.trim());
                count += 1;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("module") {
            fact.name = Some(rest.trim().trim_matches('"').to_owned());
        } else if let Some(rest) = line.strip_prefix("go ") {
            fact.meta.insert("go".to_owned(), rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("require") {
            let rest = rest.trim();
            if rest == "(" {
                in_require = true;
            } else if let Some((name, version)) = rest.split_once(char::is_whitespace) {
                add(&mut fact, name, version.trim());
                count += 1;
            }
        }
    }
    fact.meta
        .insert("require_count".to_owned(), count.to_string());
    (fact, Vec::new())
}

fn add(fact: &mut ManifestFact, name: &str, version: &str) {
    fact.dependencies.insert(
        name.to_owned(),
        DepSpec {
            range: version.to_owned(),
            kind: DepKind::Prod,
            workspace_protocol: false,
        },
    );
}
