//! POL-002: config sync at snapshot time and `config_hash`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::NaiveDate;
use profile::config::sync::{
    bind_pull_request_policy, requires_graph_rebuild, sync_config_at, MemoryTree, WorkingTree,
    REVIEW_POLICY_CHANGED,
};
use profile::config::{normalize, ConfigStatus, ReviewConfigV1, CONFIG_PATH};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
}

const BASE: &str = "version: 1\nrules:\n  forbidden_dependencies: [ { id: no-ctrl-repo, from: \
                    controllers, to: repositories, reason: use services } ]\n";

fn tree(config: Option<&str>) -> MemoryTree {
    let tree = MemoryTree::default().with("src/a.ts", "export {};\n");
    match config {
        Some(text) => tree.with(CONFIG_PATH, text),
        None => tree,
    }
}

#[test]
fn config_read_from_snapshot_tree() {
    let synced = sync_config_at(&tree(Some(BASE)), today());
    assert_eq!(synced.loaded.status, ConfigStatus::Valid);
    assert_eq!(synced.source_path.as_deref(), Some(CONFIG_PATH));
    assert_eq!(synced.loaded.config.rules.forbidden_dependencies.len(), 1);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".review")).unwrap();
    std::fs::write(dir.path().join(CONFIG_PATH), BASE).unwrap();
    let from_disk = sync_config_at(&WorkingTree::new(dir.path()), today());
    assert_eq!(from_disk.config_hash(), synced.config_hash());
}

#[test]
fn missing_config_uses_defaults_hash() {
    let synced = sync_config_at(&tree(None), today());
    assert_eq!(synced.loaded.status, ConfigStatus::Missing);
    assert_eq!(synced.source_path, None);
    assert_eq!(
        synced.config_hash(),
        normalize::config_hash(&ReviewConfigV1::default())
    );
    // An invalid file never shares the defaults' hash.
    let invalid = sync_config_at(&tree(Some("version: 1\nbogus: true\n")), today());
    assert_eq!(invalid.loaded.status, ConfigStatus::Invalid);
    assert_ne!(invalid.config_hash(), synced.config_hash());
    assert_eq!(invalid.loaded.config, ReviewConfigV1::default());
}

#[test]
fn pr_uses_base_policy() {
    let base = sync_config_at(&tree(Some(BASE)), today()).loaded;
    let head = sync_config_at(&tree(Some("version: 1\n")), today()).loaded;
    let policy = bind_pull_request_policy(&base, &head);
    assert_eq!(policy.effective.config_hash, base.config_hash);
    assert_eq!(
        policy.effective.config.rules.forbidden_dependencies.len(),
        1
    );
    let change = policy.change.as_ref().unwrap();
    assert_eq!(change.rules_removed, vec!["no-ctrl-repo"]);
    assert!(change
        .summary_line()
        .contains("This PR changes review policy"));

    let unchanged = bind_pull_request_policy(&base, &base);
    assert!(unchanged.change.is_none());
    assert!(unchanged.risk_signals().is_empty());
}

#[test]
fn pr_adding_suppression_flagged_not_applied() {
    let base = sync_config_at(&tree(Some(BASE)), today()).loaded;
    let head_text = format!(
        "{BASE}suppressions: [ {{ id: hide-mine, type: path, value: \"src/**\", reason: mine }} ]\n"
    );
    let head = sync_config_at(&tree(Some(&head_text)), today()).loaded;
    assert_eq!(head.status, ConfigStatus::Valid);
    let policy = bind_pull_request_policy(&base, &head);
    assert!(policy.effective.applicable_suppressions().is_empty());
    let change = policy.change.as_ref().unwrap();
    assert_eq!(change.suppressions_added, vec!["hide-mine"]);
    assert!(change.changed_sections.contains(&"suppressions".to_owned()));
    assert_eq!(policy.risk_signals(), vec![REVIEW_POLICY_CHANGED]);
}

#[test]
fn rule_only_change_does_not_rebuild_graph() {
    let base = sync_config_at(&tree(Some(BASE)), today()).loaded;
    let head = sync_config_at(
        &tree(Some(
            "version: 1\nrules:\n  queue_jobs: { require_deterministic_id: true }\n",
        )),
        today(),
    )
    .loaded;
    assert_ne!(base.config_hash, head.config_hash);
    assert!(!requires_graph_rebuild(&base, &head));
}

#[test]
fn generated_glob_change_triggers_rebuild() {
    let base = sync_config_at(&tree(Some(BASE)), today()).loaded;
    let head_text = format!("{BASE}generated:\n  include: [\"src/gen/**\"]\n");
    let head = sync_config_at(&tree(Some(&head_text)), today()).loaded;
    assert!(requires_graph_rebuild(&base, &head));
    let review_ignore = format!("{BASE}review:\n  generated: {{ ignore: [\"**/*.gen.ts\"] }}\n");
    let head = sync_config_at(&tree(Some(&review_ignore)), today()).loaded;
    assert!(requires_graph_rebuild(&base, &head));
}
