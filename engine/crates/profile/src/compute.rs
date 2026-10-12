//! Computing a profile for one snapshot: inference (PROF-002), mining (PROF-003..005) and the
//! documentation rules (PROF-007), under the `profile_compute` span.

use std::collections::BTreeMap;
use std::time::Instant;

use chrono::{DateTime, Utc};
use codegraph::NodeKind;

use crate::conventions::{ConventionRegistry, MiningContext};
use crate::infer::architecture::{infer_architecture, round3};
use crate::infer::roles::{RoleIndex, RoleSource};
use crate::infer::ProfileInputs;
use crate::model::{
    DocumentationRuleRef, LanguageShare, ProfileKey, RepositoryProfile, SuffixCount,
};

/// Computes the profile of `key` from `inputs`. Deterministic for the same inputs and
/// `computed_at`.
pub fn compute_profile(
    key: ProfileKey,
    inputs: ProfileInputs<'_>,
    registry: &ConventionRegistry,
    documentation_rules: Vec<DocumentationRuleRef>,
    computed_at: DateTime<Utc>,
) -> RepositoryProfile {
    let span = tracing::info_span!(
        "profile_compute",
        repository_id = %key.repository_id,
        snapshot_id = %key.snapshot_id,
        profile_version = key.profile_version.0,
        profile_layers_total = tracing::field::Empty,
        profile_modules_total = tracing::field::Empty,
        conventions_total = tracing::field::Empty,
    );
    let _guard = span.enter();
    let started = Instant::now();

    let module_files: Vec<String> = inputs
        .module_declarations
        .iter()
        .map(|d| d.file.clone())
        .collect();
    let roles = RoleIndex::build(inputs.graph, &module_files);
    let architecture = infer_architecture(&inputs, &roles);
    let modules = architecture.module_index();
    let cx = MiningContext::new(inputs, &roles, &modules);

    let mut profile = RepositoryProfile::empty(key, computed_at);
    profile.languages = languages(&inputs);
    profile.naming.file_suffixes = suffixes(&roles);
    profile.persistence.entities = count(&inputs, NodeKind::DatabaseEntity);
    profile.persistence.tables = count(&inputs, NodeKind::DatabaseTable);
    profile.persistence.migration_paths = inputs.config.rules.database.migration_paths.clone();
    profile.conventions = registry.mine_all(&cx, key.profile_version, computed_at);
    registry.contribute_all(&cx, &mut profile);
    profile.architecture = architecture.model;
    profile.modules = architecture.modules;
    profile.notes = architecture.notes;
    let mut docs = documentation_rules;
    docs.sort_by(|a, b| a.id.cmp(&b.id));
    profile.documentation_rules = docs;

    span.record("profile_layers_total", profile.architecture.layers.len());
    span.record("profile_modules_total", profile.modules.len());
    span.record("conventions_total", profile.conventions.len());
    crate::metrics::profile_compute_duration(started.elapsed().as_secs_f64());
    profile
}

fn count(inputs: &ProfileInputs<'_>, kind: NodeKind) -> u32 {
    let mut n = 0u32;
    inputs.graph.for_each_node(&mut |node| {
        if node.kind == kind {
            n += 1;
        }
    });
    n
}

fn languages(inputs: &ProfileInputs<'_>) -> Vec<LanguageShare> {
    let graph = inputs.graph;
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    graph.for_each_node(&mut |node| {
        if let Some(path) = node.file {
            if !files.contains_key(path) {
                if let Some(file) = graph.file(path) {
                    files.insert(path.to_owned(), file.language.as_str().to_owned());
                }
            }
        }
    });
    let total = files.len();
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for language in files.into_values() {
        *counts.entry(language).or_default() += 1;
    }
    let mut out: Vec<LanguageShare> = counts
        .into_iter()
        .map(|(language, files)| LanguageShare {
            language,
            files,
            share: if total == 0 {
                0.0
            } else {
                round3(files as f32 / total as f32)
            },
        })
        .collect();
    out.sort_by(|a, b| b.files.cmp(&a.files).then(a.language.cmp(&b.language)));
    out
}

fn suffixes(roles: &RoleIndex) -> Vec<SuffixCount> {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for (_, assignment) in roles.files() {
        if assignment.source == RoleSource::Suffix {
            *counts.entry(assignment.role.as_str().to_owned()).or_default() += 1;
        }
    }
    let mut out: Vec<SuffixCount> = counts
        .into_iter()
        .map(|(suffix, files)| SuffixCount { suffix, files })
        .collect();
    out.sort_by(|a, b| b.files.cmp(&a.files).then(a.suffix.cmp(&b.suffix)));
    out
}
