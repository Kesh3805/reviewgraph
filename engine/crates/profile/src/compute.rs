//! The `profile_compute` stage (PROF-001..PROF-005): graph in, [`RepositoryProfile`] out.
//!
//! Pure over an immutable graph view and deterministic for a given snapshot. A pull request never
//! runs it: the head reuses the base snapshot's profile (PROF-006, see [`crate::cache`]).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use chrono::{DateTime, Utc};
use codegraph::NodeKind;
use review_core::ids::{RepositoryId, SnapshotId};
use review_core::version::ConfigHash;

use crate::conventions::builtin::{api, error_hierarchy, guard_usage, queues, testing};
use crate::conventions::{MineOptions, MinerRegistry};
use crate::infer::infer_architecture;
use crate::infer::roles::suffix_match;
use crate::metrics;
use crate::model::{
    DocumentationRuleRef, FrameworkFact, LanguageShare, NamingProfile, RepositoryProfile,
    SecurityProfile,
};
use crate::version::{profile_version, INFERENCE_VERSION};

/// Everything the computation needs besides the graph.
#[derive(Debug, Clone)]
pub struct ComputeInput {
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub config_hash: ConfigHash,
    pub computed_at: DateTime<Utc>,
    pub options: MineOptions,
    /// Detected frameworks (from repository init facts).
    pub frameworks: Vec<FrameworkFact>,
    /// Documentation rules ingested from knowledge sources (PROF-007).
    pub documentation_rules: Vec<DocumentationRuleRef>,
}

/// The profile version of this build: a hash of every miner version and the inference version.
pub fn current_profile_version() -> u32 {
    profile_version(&MinerRegistry::builtin().versions(), INFERENCE_VERSION)
}

/// Computes the profile of the snapshot behind `g`.
pub fn compute_profile(
    g: &dyn crate::conventions::GraphView,
    input: &ComputeInput,
) -> RepositoryProfile {
    let started = Instant::now();
    let span = tracing::info_span!(
        "profile_compute",
        profile_layers_total = tracing::field::Empty,
        profile_modules_total = tracing::field::Empty
    );
    let _guard = span.enter();

    let q = g.graph();
    let mut profile = RepositoryProfile::empty(
        input.repository_id,
        input.snapshot_id,
        input.config_hash,
        input.computed_at,
    );
    let registry = MinerRegistry::builtin();
    profile.profile_version = profile_version(&registry.versions(), INFERENCE_VERSION);

    let inferred = infer_architecture(q);
    span.record(
        "profile_layers_total",
        inferred.architecture.layers.len() as u64,
    );
    span.record("profile_modules_total", inferred.modules.len() as u64);
    profile.architecture = inferred.architecture;
    profile.modules = inferred.modules;

    // Languages and naming come from the file table.
    let mut languages: BTreeMap<String, u32> = BTreeMap::new();
    let mut suffixes: BTreeSet<String> = BTreeSet::new();
    let mut files = 0u32;
    q.for_each_node(&mut |n| {
        if n.kind != NodeKind::File {
            return;
        }
        files += 1;
        if let Some(file) = n.file.and_then(|p| q.file(p)) {
            *languages.entry(file.language.to_string()).or_insert(0) += 1;
        }
        if let Some((tag, _)) = n.file.and_then(suffix_match) {
            suffixes.insert(format!(".{tag}."));
        }
    });
    profile.languages = languages
        .into_iter()
        .map(|(language, count)| LanguageShare {
            language,
            files: count,
            share: if files == 0 {
                0.0
            } else {
                count as f32 / files as f32
            },
        })
        .collect();
    profile.naming = NamingProfile {
        file_suffixes: suffixes.into_iter().collect(),
        case_style: None,
    };
    profile.frameworks = input.frameworks.clone();
    profile.documentation_rules = input.documentation_rules.clone();

    // Sections that read the graph.
    profile.testing = testing::infer_testing(g);
    profile.api = api::infer_api(g);
    profile.queues = queues::infer_queues(g);
    profile.security = SecurityProfile {
        guard_symbols: crate::conventions::builtin::guard_names(g),
        authorization_symbols: guard_usage::authorization_symbols(g),
    };
    profile.errors.base_errors = error_hierarchy::analyze(g).base.into_iter().collect();

    // Conventions last: they may read the sections above.
    let conventions = registry.mine_all(g, &profile, &input.options, input.computed_at);
    profile.conventions = conventions;

    metrics::profile_compute_duration(started.elapsed().as_secs_f64());
    profile
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use codegraph::EdgeKind;

    use super::*;
    use crate::testgraph::{OwnedView, TestGraph};

    fn input() -> ComputeInput {
        ComputeInput {
            repository_id: RepositoryId::new(),
            snapshot_id: SnapshotId::new(),
            config_hash: ConfigHash::of(b"cfg"),
            computed_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            options: MineOptions::default(),
            frameworks: vec![],
            documentation_rules: vec![],
        }
    }

    fn graph() -> OwnedView {
        let mut t = TestGraph::new();
        for m in ["a", "b", "c"] {
            let (ctrl, svc) = (format!("f:{m}.ctrl"), format!("f:{m}.svc"));
            let (cp, sp) = (
                format!("src/{m}/{m}.controller.ts"),
                format!("src/{m}/{m}.service.ts"),
            );
            t.node(NodeKind::File, &ctrl, "c.ts", Some(cp.as_str()))
                .node(NodeKind::File, &svc, "s.ts", Some(sp.as_str()))
                .edge(EdgeKind::Imports, &ctrl, &svc);
        }
        OwnedView::new(t.build())
    }

    #[test]
    fn profile_is_deterministic_and_versioned() {
        let (a, b) = (
            compute_profile(&graph(), &input()),
            compute_profile(&graph(), &input()),
        );
        assert_eq!(a.profile_version, current_profile_version());
        assert_eq!(a.architecture, b.architecture);
        assert_eq!(a.conventions, b.conventions);
        assert_eq!(a.architecture.layers.len(), 2);
        assert!(a.naming.file_suffixes.contains(&".controller.".to_owned()));
    }

    #[test]
    fn conventions_carry_the_computed_profile_version() {
        let p = compute_profile(&graph(), &input());
        assert!(p
            .conventions
            .iter()
            .all(|c| c.profile_version == p.profile_version));
    }
}
