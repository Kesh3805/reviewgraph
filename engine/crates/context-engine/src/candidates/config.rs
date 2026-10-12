//! Configuration and environment candidates (CTX-003).
//!
//! Excerpts are key level: key names and line numbers. Values appear only for non-secret
//! configuration files and never for `.env*` files or keys that look like secrets.

use impact::Relation;
use review_core::location::{DiffSide, RepoPath};
use serde_json::json;

use super::{
    convert_path, Candidate, CandidateGenerator, CandidatePool, GenStats, LineSpan, SelectionInput,
};
use crate::model::{CandidateSource, ItemKind};

/// Longest value kept in an excerpt.
const MAX_VALUE_CHARS: usize = 64;
const SECRET_HINTS: [&str; 7] = [
    "secret",
    "password",
    "passwd",
    "token",
    "credential",
    "private",
    "apikey",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigKey {
    pub key: String,
    pub line: u32,
    pub value: Option<String>,
}

/// A configuration file changed in the cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    pub path: RepoPath,
    pub keys: Vec<ConfigKey>,
}

fn is_env_file(path: &RepoPath) -> bool {
    path.as_str()
        .rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with(".env"))
}

fn looks_secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase().replace(['_', '-', '.'], "");
    SECRET_HINTS.iter().any(|h| k.contains(h)) || k.ends_with("key")
}

/// The value to show for a key, if any.
pub fn visible_value(file: &ConfigFile, key: &ConfigKey) -> Option<String> {
    if is_env_file(&file.path) || looks_secret(&key.key) {
        return None;
    }
    key.value
        .as_ref()
        .map(|v| v.chars().take(MAX_VALUE_CHARS).collect())
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConfigCandidates;

impl CandidateGenerator for ConfigCandidates {
    fn source(&self) -> CandidateSource {
        CandidateSource::Config
    }

    fn generate(&self, input: &SelectionInput<'_>, out: &mut CandidatePool) -> GenStats {
        let mut stats = GenStats::default();
        for file in &input.config_files {
            let keys: Vec<_> = file
                .keys
                .iter()
                .map(|k| {
                    json!({
                        "key": k.key,
                        "line": k.line,
                        "value": visible_value(file, k),
                    })
                })
                .collect();
            let start = file.keys.iter().map(|k| k.line).min().unwrap_or(1);
            let end = file.keys.iter().map(|k| k.line).max().unwrap_or(start);
            let mut c = Candidate::named(
                file.path.as_str(),
                ItemKind::Config,
                DiffSide::Head,
                CandidateSource::Config,
            );
            c.distance = Some(1);
            c.raw.config_relationship = 1.0;
            c.structured = Some(json!({ "file": file.path.as_str(), "keys": keys }));
            c.span = Some(LineSpan {
                file: file.path.clone(),
                start_line: start,
                end_line: end,
                side: DiffSide::Head,
            });
            stats.record(out.insert(c));
        }
        let Some(graph) = input.impact else {
            stats.degraded = true;
            return stats;
        };
        let seeds = input.seed_keys();
        for impact in graph
            .symbols
            .iter()
            .filter(|i| seeds.contains(&i.seed) && i.skipped.is_none())
        {
            for el in impact
                .elements
                .iter()
                .filter(|e| matches!(e.relation, Relation::Config | Relation::EnvVar))
            {
                let info = input
                    .locator
                    .and_then(|l| l.locate(el.node, DiffSide::Head));
                let title = info
                    .as_ref()
                    .map_or_else(|| el.node_id.clone(), |i| i.title.clone());
                let mut c = Candidate::node(
                    el.node,
                    title,
                    ItemKind::Config,
                    DiffSide::Head,
                    CandidateSource::Config,
                );
                c.distance = Some(el.distance);
                c.path = convert_path(&el.path);
                c.min_confidence = el.min_confidence.as_f32();
                c.relation = Some(el.relation.as_str().to_owned());
                c.weak = el.weak;
                // Read directly by a changed symbol: 1.0; by a caller or further away: 0.5.
                c.raw.config_relationship = if el.distance <= 1 { 1.0 } else { 0.5 };
                c.structured = Some(json!({ "key": el.node_id, "kind": el.relation.as_str() }));
                c.span = info.map(|i| LineSpan {
                    file: i.file,
                    start_line: i.start_line,
                    end_line: i.end_line,
                    side: DiffSide::Head,
                });
                stats.record(out.insert(c));
            }
        }
        stats
    }
}
