//! Maven (streamed) and Gradle (settings files only; build files are recorded).

use std::sync::OnceLock;

use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use review_core::location::RepoPath;

use super::{DepKind, DepSpec, Ecosystem, ManifestFact, ParseStatusLite};
use crate::error::InitWarning;

/// Bytes of a pom.xml that are read.
const POM_LIMIT: usize = 1024 * 1024;

fn truncate_on_char_boundary(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub(crate) fn parse_pom(path: &RepoPath, text: &str) -> (ManifestFact, Vec<InitWarning>) {
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Maven);
    let mut warnings = Vec::new();
    let mut reader = Reader::from_str(truncate_on_char_boundary(text, POM_LIMIT));
    reader.config_mut().trim_text(true);

    let mut stack: Vec<String> = Vec::new();
    let (mut group, mut artifact, mut version, mut parent_group) = (None, None, None, None);
    let mut dep_group: Option<String> = None;
    let mut dep_artifact: Option<String> = None;
    let mut dep_version: Option<String> = None;
    let mut dep_scope: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                stack.push(String::from_utf8_lossy(e.local_name().as_ref()).into_owned());
            }
            Ok(Event::End(_)) => {
                let closing = stack.pop();
                if closing.as_deref() == Some("dependency") && stack == ["project", "dependencies"]
                {
                    if let (Some(g), Some(a)) = (dep_group.take(), dep_artifact.take()) {
                        let kind = match dep_scope.take().as_deref() {
                            Some("test") => DepKind::Dev,
                            Some("provided") => DepKind::Peer,
                            Some("optional") => DepKind::Optional,
                            _ => DepKind::Prod,
                        };
                        fact.dependencies.insert(
                            format!("{g}:{a}"),
                            DepSpec {
                                range: dep_version.take().unwrap_or_default(),
                                kind,
                                workspace_protocol: false,
                            },
                        );
                    }
                    dep_group = None;
                    dep_artifact = None;
                    dep_version = None;
                    dep_scope = None;
                }
            }
            Ok(Event::Text(t)) => {
                let Ok(value) = t.unescape() else { continue };
                let value = value.trim().to_owned();
                let s: Vec<&str> = stack.iter().map(String::as_str).collect();
                match s.as_slice() {
                    ["project", "groupId"] => group = Some(value),
                    ["project", "artifactId"] => artifact = Some(value),
                    ["project", "version"] => version = Some(value),
                    ["project", "parent", "groupId"] => parent_group = Some(value),
                    ["project", "modules", "module"] => fact.members.push(value),
                    ["project", "dependencies", "dependency", "groupId"] => dep_group = Some(value),
                    ["project", "dependencies", "dependency", "artifactId"] => {
                        dep_artifact = Some(value)
                    }
                    ["project", "dependencies", "dependency", "version"] => {
                        dep_version = Some(value)
                    }
                    ["project", "dependencies", "dependency", "scope"] => dep_scope = Some(value),
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => {
                warnings.push(InitWarning::new(
                    "manifest_parse",
                    Some(path.clone()),
                    format!(
                        "pom.xml is not well-formed XML (byte {})",
                        reader.error_position()
                    ),
                ));
                fact.parse_status = ParseStatusLite::Partial;
                break;
            }
            _ => {}
        }
    }
    let group = group.or(parent_group);
    fact.name = match (group, artifact) {
        (Some(g), Some(a)) => Some(format!("{g}:{a}")),
        (None, Some(a)) => Some(a),
        _ => None,
    };
    fact.version = version;
    (fact, warnings)
}

fn regexes() -> &'static Option<(Regex, Regex, Regex)> {
    static RE: OnceLock<Option<(Regex, Regex, Regex)>> = OnceLock::new();
    RE.get_or_init(|| {
        Some((
            Regex::new(r#"rootProject\.name\s*=\s*["']([^"']+)["']"#).ok()?,
            Regex::new(r#"include\s*\(?([^)\n]*)"#).ok()?,
            Regex::new(r#"["']:?([^"']+)["']"#).ok()?,
        ))
    })
}

pub(crate) fn parse_gradle(
    path: &RepoPath,
    lower_name: &str,
    text: &str,
) -> (ManifestFact, Vec<InitWarning>) {
    let mut fact = ManifestFact::new(path.clone(), Ecosystem::Gradle);
    if !lower_name.starts_with("settings.gradle") {
        return (fact, Vec::new());
    }
    let Some((root_name, include, quoted)) = regexes() else {
        return (fact, Vec::new());
    };
    fact.name = root_name
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned());
    for caps in include.captures_iter(text) {
        let args = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        for q in quoted.captures_iter(args) {
            if let Some(m) = q.get(1) {
                fact.members.push(m.as_str().to_owned());
            }
        }
    }
    (fact, Vec::new())
}
