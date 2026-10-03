//! Framework detection from manifests and config files (INIT-006).
//!
//! Source-level detection is the adapters' job (they can raise or lower these confidences).

pub mod rules;

use std::collections::BTreeMap;

use globset::{Glob, GlobMatcher};
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::manifests::{DepKind, Ecosystem, ManifestFact, ManifestFacts};
use crate::walk::FileInventory;
use rules::{confidence, RULES, VIA_RULES};

pub type FrameworkId = String;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkCategory {
    Web,
    Ui,
    Orm,
    Queue,
    Test,
    Auth,
    Validation,
    Config,
    Docs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceSource {
    Dependency { dep_kind: DepKind },
    ConfigFile,
    SchemaFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameworkEvidence {
    pub source: EvidenceSource,
    pub path: RepoPath,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameworkFact {
    pub id: FrameworkId,
    pub category: FrameworkCategory,
    /// Package directory (the repository root is the empty string).
    pub scope: RepoDir,
    pub version_range: Option<String>,
    pub major: Option<u64>,
    pub confidence: f32,
    pub evidence: Vec<FrameworkEvidence>,
    /// The host framework when only a wrapper package was found.
    pub via: Option<FrameworkId>,
}

/// Auth libraries discovered at init. Route-level boundaries come from NEST-004 at index time.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct AuthFacts {
    pub libraries: Vec<FrameworkId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameworkPresence {
    pub scope_dirs: Vec<RepoDir>,
    pub major: Option<u64>,
    pub confidence: f32,
}

/// Plain-data framework signals. The crate DAG forbids `repository` -> `analysis-ir`, so the
/// composition root maps this field-for-field into `analysis_ir::FrameworkSignals`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct FrameworkSignalsData {
    pub frameworks: BTreeMap<FrameworkId, FrameworkPresence>,
}

/// The major version of a range such as `^10.3.0`, `~5`, `>=2 <3` or `workspace:^1.2.3`.
/// Unparseable ranges (`latest`, git URLs) give `None`.
pub fn parse_major(range: &str) -> Option<u64> {
    let range = range
        .trim()
        .strip_prefix("workspace:")
        .unwrap_or(range.trim());
    let range = range.trim_start_matches(|c: char| "^~>=< v".contains(c));
    let token = range
        .split(|c: char| c.is_whitespace() || c == ',' || c == '|')
        .next()?;
    token.split('.').next()?.parse().ok()
}

struct CompiledRule {
    matchers: Vec<(GlobMatcher, bool)>, // (matcher, is_schema)
}

fn matcher(pattern: &str) -> Option<GlobMatcher> {
    Glob::new(pattern).ok().map(|g| g.compile_matcher())
}

fn compile(rule: &rules::Rule) -> CompiledRule {
    let mut matchers = Vec::new();
    for g in rule.config_globs {
        if let Some(m) = matcher(g) {
            matchers.push((m, false));
        }
    }
    for g in rule.schema_globs {
        if let Some(m) = matcher(g) {
            matchers.push((m, true));
        }
    }
    CompiledRule { matchers }
}

/// Path relative to `dir` (dir is a prefix of path).
fn relative<'a>(dir: &RepoDir, path: &'a str) -> &'a str {
    if dir.is_root() {
        path
    } else {
        path[dir.as_str().len()..].trim_start_matches('/')
    }
}

fn dep_evidence(manifest: &ManifestFact, dep: &str) -> Option<FrameworkEvidence> {
    let spec = manifest.dependencies.get(dep)?;
    Some(FrameworkEvidence {
        source: EvidenceSource::Dependency {
            dep_kind: spec.kind,
        },
        path: manifest.path.clone(),
        detail: format!("{dep}@{}", spec.range),
    })
}

/// Detects frameworks for every npm package in `manifests`.
pub fn detect_frameworks(
    inventory: &FileInventory,
    manifests: &ManifestFacts,
) -> (Vec<FrameworkFact>, Vec<InitWarning>) {
    let span = tracing::info_span!("init.frameworks", frameworks.ids = tracing::field::Empty);
    let _guard = span.enter();

    let npm: Vec<&ManifestFact> = manifests
        .manifests
        .iter()
        .filter(|m| m.ecosystem == Ecosystem::Npm)
        .collect();
    let compiled: Vec<CompiledRule> = RULES.iter().map(compile).collect();

    // Assign every file to the nearest enclosing package directory.
    let mut dirs: Vec<RepoDir> = npm.iter().map(|m| m.dir()).collect();
    dirs.sort();
    dirs.dedup();
    let owner_of = |path: &str| -> RepoDir {
        dirs.iter()
            .filter(|d| d.contains(path))
            .max_by_key(|d| d.depth())
            .cloned()
            .unwrap_or_else(RepoDir::root)
    };
    let mut files_by_scope: BTreeMap<RepoDir, Vec<&RepoPath>> = BTreeMap::new();
    for entry in &inventory.entries {
        files_by_scope
            .entry(owner_of(entry.path.as_str()))
            .or_default()
            .push(&entry.path);
    }

    let mut facts: BTreeMap<(RepoDir, String), FrameworkFact> = BTreeMap::new();
    let mut scopes: Vec<RepoDir> = dirs.clone();
    for scope in files_by_scope.keys() {
        if !scopes.contains(scope) {
            scopes.push(scope.clone());
        }
    }
    scopes.sort();

    for scope in &scopes {
        let manifest = npm.iter().copied().find(|m| m.dir() == *scope);
        let files = files_by_scope.get(scope).map(Vec::as_slice).unwrap_or(&[]);
        for (rule, compiled) in RULES.iter().zip(&compiled) {
            let mut evidence = Vec::new();
            let mut version_range = None;
            let mut best_dep_kind: Option<DepKind> = None;
            if let Some(manifest) = manifest {
                for dep in rule.deps {
                    if let Some(ev) = dep_evidence(manifest, dep) {
                        if version_range.is_none() {
                            version_range =
                                manifest.dependencies.get(*dep).map(|s| s.range.clone());
                        }
                        if let EvidenceSource::Dependency { dep_kind } = ev.source {
                            best_dep_kind = Some(match best_dep_kind {
                                Some(DepKind::Prod) => DepKind::Prod,
                                _ => dep_kind,
                            });
                        }
                        evidence.push(ev);
                    }
                }
                if rule.id == "jest" && manifest.npm.as_ref().is_some_and(|n| n.has_jest_key) {
                    evidence.push(FrameworkEvidence {
                        source: EvidenceSource::ConfigFile,
                        path: manifest.path.clone(),
                        detail: "package.json `jest` key".to_owned(),
                    });
                }
            }
            let has_dep = best_dep_kind.is_some();
            let mut has_config = evidence
                .iter()
                .any(|e| !matches!(e.source, EvidenceSource::Dependency { .. }));
            for path in files {
                let rel = relative(scope, path.as_str());
                for (m, is_schema) in &compiled.matchers {
                    if m.is_match(rel) {
                        evidence.push(FrameworkEvidence {
                            source: if *is_schema {
                                EvidenceSource::SchemaFile
                            } else {
                                EvidenceSource::ConfigFile
                            },
                            path: (*path).clone(),
                            detail: "config file".to_owned(),
                        });
                        has_config = true;
                        break;
                    }
                }
            }
            if evidence.is_empty() {
                continue;
            }
            let conf = match (has_dep, has_config) {
                (true, true) => confidence::DEP_AND_CONFIG,
                (false, true) => confidence::CONFIG_ONLY,
                (true, false) => {
                    if rule.category == FrameworkCategory::Test {
                        confidence::TEST_DEP
                    } else if rule.runtime && best_dep_kind == Some(DepKind::Dev) {
                        confidence::RUNTIME_DEV_ONLY
                    } else {
                        confidence::RUNTIME_PROD
                    }
                }
                (false, false) => continue,
            };
            evidence.sort_by(|a, b| (&a.path, &a.detail).cmp(&(&b.path, &b.detail)));
            let major = version_range.as_deref().and_then(parse_major);
            facts.insert(
                (scope.clone(), rule.id.to_owned()),
                FrameworkFact {
                    id: rule.id.to_owned(),
                    category: rule.category,
                    scope: scope.clone(),
                    version_range,
                    major,
                    confidence: conf,
                    evidence,
                    via: None,
                },
            );
        }

        // Wrapper packages: only reported as a framework `via` the host when the framework
        // itself is not a direct dependency.
        if let Some(manifest) = manifest {
            for (wrapper, id, host) in VIA_RULES {
                if facts.contains_key(&(scope.clone(), (*id).to_owned())) {
                    continue;
                }
                let Some(ev) = dep_evidence(manifest, wrapper) else {
                    continue;
                };
                let Some(rule) = RULES.iter().find(|r| r.id == *id) else {
                    continue;
                };
                let range = manifest.dependencies.get(*wrapper).map(|s| s.range.clone());
                facts.insert(
                    (scope.clone(), (*id).to_owned()),
                    FrameworkFact {
                        id: (*id).to_owned(),
                        category: rule.category,
                        scope: scope.clone(),
                        major: range.as_deref().and_then(parse_major),
                        version_range: range,
                        confidence: confidence::VIA,
                        evidence: vec![ev],
                        via: Some((*host).to_owned()),
                    },
                );
            }
        }
    }

    let out: Vec<FrameworkFact> = facts.into_values().collect();
    let mut ids: Vec<&str> = out.iter().map(|f| f.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    let joined: String = ids.join(",").chars().take(512).collect();
    span.record("frameworks.ids", joined.as_str());
    (out, Vec::new())
}

pub fn auth_facts(frameworks: &[FrameworkFact]) -> AuthFacts {
    let mut libraries: Vec<FrameworkId> = frameworks
        .iter()
        .filter(|f| f.category == FrameworkCategory::Auth)
        .map(|f| f.id.clone())
        .collect();
    libraries.sort();
    libraries.dedup();
    AuthFacts { libraries }
}

/// Collapses per-package facts into the signal map adapters consume.
pub fn to_framework_signals(frameworks: &[FrameworkFact]) -> FrameworkSignalsData {
    let mut map: BTreeMap<FrameworkId, FrameworkPresence> = BTreeMap::new();
    for fact in frameworks {
        let entry = map
            .entry(fact.id.clone())
            .or_insert_with(|| FrameworkPresence {
                scope_dirs: Vec::new(),
                major: fact.major,
                confidence: fact.confidence,
            });
        entry.scope_dirs.push(fact.scope.clone());
        if fact.confidence > entry.confidence {
            entry.confidence = fact.confidence;
            entry.major = fact.major.or(entry.major);
        }
        if entry.major.is_none() {
            entry.major = fact.major;
        }
    }
    for presence in map.values_mut() {
        presence.scope_dirs.sort();
        presence.scope_dirs.dedup();
    }
    FrameworkSignalsData { frameworks: map }
}

#[cfg(test)]
mod tests {
    use super::parse_major;

    #[test]
    fn major_parsing() {
        assert_eq!(parse_major("^10.3.0"), Some(10));
        assert_eq!(parse_major("~5"), Some(5));
        assert_eq!(parse_major(">=2 <3"), Some(2));
        assert_eq!(parse_major("workspace:^1.2.3"), Some(1));
        assert_eq!(parse_major("latest"), None);
        assert_eq!(parse_major("git+https://x/y.git"), None);
        assert_eq!(parse_major("workspace:*"), None);
    }
}
