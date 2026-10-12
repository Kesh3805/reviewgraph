#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! INC-011 acceptance tests over the real snapshot and change-set types.

use std::collections::BTreeMap;

use chrono::Utc;
use graph_storage::status::SnapshotStatus;
use graph_storage::types::{
    SnapshotKind, SnapshotMeta, SnapshotPurpose, SnapshotStats, SnapshotVersions,
};
use incremental::changeset::{ChangeSet, ChangeSetStats, PathChange, PathChangeKind};
use incremental::rebuild::{
    evaluate_snapshot, touched_config_paths, DecisionContext, FullMode, RebuildDecision,
    RebuildFlags, RebuildReason,
};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};

fn versions() -> SnapshotVersions {
    let mut v = SnapshotVersions::default();
    v.analyzer_versions
        .insert("typescript".to_owned(), "1.0.0".to_owned());
    v.analyzer_versions
        .insert("linker".to_owned(), "l1".to_owned());
    v.config_components
        .insert("tsconfig".to_owned(), "h1".to_owned());
    v.config_components
        .insert("source_roots".to_owned(), "r1".to_owned());
    v
}

fn meta(status: SnapshotStatus, depth: u16) -> SnapshotMeta {
    let now = Utc::now();
    SnapshotMeta {
        id: SnapshotId::new(),
        scope: RepoScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        },
        commit_sha: "a".repeat(40).parse().unwrap(),
        kind: SnapshotKind::Full,
        base: None,
        chain_depth: depth,
        purpose: SnapshotPurpose::DefaultBranch,
        status,
        versions: versions(),
        stats: SnapshotStats::default(),
        error: None,
        created_at: now,
        updated_at: now,
        completed_at: Some(now),
    }
}

fn changes(paths: &[&str]) -> ChangeSet {
    let entries: Vec<PathChange> = paths
        .iter()
        .map(|p| PathChange {
            path: p.parse().unwrap(),
            kind: PathChangeKind::Modified,
            renamed_from: None,
            base_oid: None,
            head_oid: None,
            similarity: None,
        })
        .collect();
    ChangeSet {
        base: "a".repeat(40).parse().unwrap(),
        head: "b".repeat(40).parse().unwrap(),
        stats: ChangeSetStats {
            entries: entries.len() as u32,
            ..ChangeSetStats::default()
        },
        entries,
    }
}

fn ctx(languages: &[String], flags: RebuildFlags) -> DecisionContext<'_> {
    DecisionContext {
        languages,
        base_file_count: 100,
        max_change_ratio: 0.5,
        max_chain_depth: 20,
        delta_edge_churn: 0.0,
        max_edge_churn: 0.1,
        flags,
    }
}

fn decide(
    base: &SnapshotMeta,
    head: &SnapshotVersions,
    head_config: Option<&BTreeMap<String, String>>,
    set: &ChangeSet,
    flags: RebuildFlags,
) -> RebuildDecision {
    let languages = vec!["typescript".to_owned()];
    evaluate_snapshot(base, head, head_config, set, &ctx(&languages, flags))
}

fn reasons(decision: RebuildDecision) -> Vec<RebuildReason> {
    match decision {
        RebuildDecision::Full { reasons, .. } => reasons,
        other => panic!("expected a full rebuild, got {other:?}"),
    }
}

#[test]
fn same_versions_and_small_change_is_incremental() {
    let base = meta(SnapshotStatus::Ready, 1);
    let d = decide(
        &base,
        &versions(),
        None,
        &changes(&["src/a.ts"]),
        RebuildFlags::default(),
    );
    assert_eq!(d, RebuildDecision::Incremental);
}

#[test]
fn tsconfig_paths_change_forces_full() {
    let base = meta(SnapshotStatus::Ready, 1);
    let mut head_config = base.versions.config_components.clone();
    head_config.insert("tsconfig".to_owned(), "h2".to_owned());
    let d = decide(
        &base,
        &versions(),
        Some(&head_config),
        &changes(&["tsconfig.json"]),
        RebuildFlags::default(),
    );
    assert!(reasons(d).contains(&RebuildReason::ConfigChanged {
        component: "tsconfig".to_owned()
    }));
}

#[test]
fn tsconfig_whitespace_edit_stays_incremental() {
    let base = meta(SnapshotStatus::Ready, 1);
    // The recomputed component hashes are identical because IDX-002 normalizes whitespace.
    let head_config = base.versions.config_components.clone();
    let d = decide(
        &base,
        &versions(),
        Some(&head_config),
        &changes(&["tsconfig.json"]),
        RebuildFlags::default(),
    );
    assert_eq!(d, RebuildDecision::Incremental);
}

#[test]
fn source_roots_change_forces_full() {
    let base = meta(SnapshotStatus::Ready, 1);
    let mut head_config = base.versions.config_components.clone();
    head_config.insert("source_roots".to_owned(), "r2".to_owned());
    let d = decide(
        &base,
        &versions(),
        Some(&head_config),
        &changes(&["pnpm-workspace.yaml"]),
        RebuildFlags::default(),
    );
    assert!(reasons(d).contains(&RebuildReason::ConfigChanged {
        component: "source_roots".to_owned()
    }));
}

#[test]
fn unverifiable_config_change_is_conservative() {
    let base = meta(SnapshotStatus::Ready, 1);
    let d = decide(
        &base,
        &versions(),
        None,
        &changes(&["package.json"]),
        RebuildFlags::default(),
    );
    assert!(reasons(d).contains(&RebuildReason::ChangeTouchesSourceRoots));
}

#[test]
fn inconsistent_base_forces_full() {
    let base = meta(SnapshotStatus::Inconsistent, 1);
    let d = decide(
        &base,
        &versions(),
        None,
        &changes(&["src/a.ts"]),
        RebuildFlags::default(),
    );
    assert!(reasons(d).contains(&RebuildReason::BaseInconsistent));
}

#[test]
fn linker_bump_relinks_from_cache() {
    let base = meta(SnapshotStatus::Ready, 1);
    let mut head = versions();
    head.analyzer_versions
        .insert("linker".to_owned(), "l2".to_owned());
    let d = decide(
        &base,
        &head,
        None,
        &changes(&["src/a.ts"]),
        RebuildFlags::default(),
    );
    assert!(matches!(
        d,
        RebuildDecision::Full {
            mode: FullMode::RelinkFromCache,
            ..
        }
    ));
}

#[test]
fn deep_chain_requests_compaction_not_rebuild() {
    let base = meta(SnapshotStatus::Ready, 20);
    let d = decide(
        &base,
        &versions(),
        None,
        &changes(&["src/a.ts"]),
        RebuildFlags::default(),
    );
    assert_eq!(d, RebuildDecision::CompactThenIncremental);
}

#[test]
fn explicit_flag_wins_and_reasons_are_sorted() {
    let base = meta(SnapshotStatus::Ready, 1);
    let mut head = versions();
    head.analyzer_versions
        .insert("linker".to_owned(), "l2".to_owned());
    let flags = RebuildFlags {
        explicit: true,
        ..RebuildFlags::default()
    };
    let r = reasons(decide(&base, &head, None, &changes(&["src/a.ts"]), flags));
    assert_eq!(r.first(), Some(&RebuildReason::Explicit));
    assert_eq!(r.last(), Some(&RebuildReason::LinkerVersionChanged));
}

#[test]
fn config_paths_are_recognized() {
    let set = changes(&[
        "src/a.ts",
        "tsconfig.base.json",
        "packages/x/package.json",
        ".review/config.yaml",
    ]);
    let touched: Vec<String> = touched_config_paths(&set)
        .iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    assert_eq!(
        touched,
        vec![
            "tsconfig.base.json".to_owned(),
            "packages/x/package.json".to_owned(),
            ".review/config.yaml".to_owned()
        ]
    );
}

#[test]
fn half_the_repo_changed_forces_full() {
    let base = meta(SnapshotStatus::Ready, 1);
    let paths: Vec<String> = (0..60).map(|i| format!("src/f{i}.ts")).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let r = reasons(decide(
        &base,
        &versions(),
        None,
        &changes(&refs),
        RebuildFlags::default(),
    ));
    assert!(r
        .iter()
        .any(|reason| matches!(reason, RebuildReason::ChangeRatioTooHigh { .. })));
}
