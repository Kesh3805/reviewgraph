//! RISK-002: path, manifest, lockfile and schema signals.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use impact::input::{
    BumpKind, ChangeSet, DependencyChange, DependencyInput, FileInput, SchemaInput, SchemaKind,
};
use impact::risk::{
    detect_manifest_signals, detect_path_signals, PathRule, RiskCategory, RiskLevel, RiskPolicy,
    RiskSignal, SignalSubject,
};
use review_core::change::FileChangeStatus;
use support::{modified_file, path};

fn files(paths: &[&str]) -> ChangeSet {
    ChangeSet {
        files: paths.iter().map(|p| modified_file(p)).collect(),
        ..ChangeSet::default()
    }
}

fn rule(signals: &[RiskSignal], rule_id: &str) -> Option<RiskSignal> {
    signals.iter().find(|s| s.rule_id == rule_id).cloned()
}

fn auth_policy() -> RiskPolicy {
    RiskPolicy::default().with_paths(vec![PathRule::new(
        "src/auth/**",
        RiskLevel::Critical,
        None,
    )
    .unwrap()])
}

#[test]
fn auth_bypass_src_auth_critical_floor() {
    let signals = detect_path_signals(&files(&["src/auth/auth.service.ts"]), &auth_policy());
    let signal = rule(&signals, "path.configured").unwrap();
    assert_eq!(signal.category, RiskCategory::Authentication);
    assert_eq!(signal.weight, 0.8);
    assert_eq!(signal.floor_level, Some(RiskLevel::Critical));
    assert_eq!(
        signal.subject,
        SignalSubject::File(path("src/auth/auth.service.ts"))
    );
}

#[test]
fn first_matching_path_rule_wins() {
    let policy = RiskPolicy::default().with_paths(vec![
        PathRule::new("src/payments/legacy/**", RiskLevel::Low, None).unwrap(),
        PathRule::new("src/payments/**", RiskLevel::Critical, None).unwrap(),
    ]);
    let signals = detect_path_signals(&files(&["src/payments/legacy/a.ts"]), &policy);
    let configured: Vec<&RiskSignal> = signals
        .iter()
        .filter(|s| s.rule_id == "path.configured")
        .collect();
    assert_eq!(configured.len(), 1);
    assert_eq!(configured[0].weight, 0.1);
    assert_eq!(configured[0].floor_level, Some(RiskLevel::Low));
    assert_eq!(configured[0].category, RiskCategory::Payments);
}

#[test]
fn explicit_category_wins_over_inference() {
    let policy = RiskPolicy::default().with_paths(vec![PathRule::new(
        "src/auth/**",
        RiskLevel::High,
        Some(RiskCategory::Permissions),
    )
    .unwrap()]);
    let signals = detect_path_signals(&files(&["src/auth/a.ts"]), &policy);
    let signal = rule(&signals, "path.configured").unwrap();
    assert_eq!(signal.category, RiskCategory::Permissions);
    assert_eq!(signal.weight, 0.6);
}

#[test]
fn default_rules_without_config() {
    let change = files(&[
        "src/auth/a.ts",
        "db/migrations/001_users.sql",
        "src/billing/invoice.ts",
        "Dockerfile",
        ".ci/workflows/build.yml",
        "src/util/format.ts",
    ]);
    let signals = detect_path_signals(&change, &RiskPolicy::default());
    let for_file = |file: &str| -> Vec<String> {
        signals
            .iter()
            .filter(|s| s.subject == SignalSubject::File(path(file)))
            .map(|s| s.rule_id.clone())
            .collect()
    };
    assert_eq!(for_file("src/auth/a.ts"), vec!["path.auth"]);
    assert_eq!(
        for_file("db/migrations/001_users.sql"),
        vec!["path.migrations"]
    );
    assert_eq!(for_file("src/billing/invoice.ts"), vec!["path.payments"]);
    assert_eq!(for_file("Dockerfile"), vec!["path.infra"]);
    assert_eq!(for_file(".ci/workflows/build.yml"), vec!["path.infra"]);
    assert!(for_file("src/util/format.ts").is_empty());
    assert_eq!(rule(&signals, "path.auth").unwrap().weight, 0.5);
}

#[test]
fn configured_rules_replace_the_defaults() {
    let policy = RiskPolicy::default().with_paths(vec![PathRule::new(
        "src/other/**",
        RiskLevel::High,
        None,
    )
    .unwrap()]);
    let signals = detect_path_signals(&files(&["src/auth/a.ts"]), &policy);
    assert!(signals.is_empty());
}

#[test]
fn invalid_glob_is_rejected() {
    assert!(PathRule::new("src/[auth", RiskLevel::High, None).is_err());
}

#[test]
fn policy_hash_changes_with_policy() {
    let a = RiskPolicy::default().policy_hash();
    let b = auth_policy().policy_hash();
    assert_eq!(a.len(), 64);
    assert_ne!(a, b);
    assert_eq!(b, auth_policy().policy_hash());
}

fn dependency(change: DependencyChange, lockfile_only: bool) -> DependencyInput {
    DependencyInput {
        name: "left-pad".to_owned(),
        manifest: path("package.json"),
        change,
        lockfile_only,
    }
}

fn with_dependencies(dependencies: Vec<DependencyInput>) -> ChangeSet {
    ChangeSet {
        dependencies,
        ..ChangeSet::default()
    }
}

#[test]
fn npm_major_bump_signal() {
    let signals = detect_manifest_signals(&with_dependencies(vec![
        dependency(DependencyChange::Bumped(BumpKind::Major), false),
        dependency(DependencyChange::Bumped(BumpKind::Patch), false),
    ]));
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].rule_id, "dep.major_bump");
    assert_eq!(signals[0].category, RiskCategory::DependencyUpdates);
    assert_eq!(signals[0].weight, 0.45);
}

#[test]
fn added_removed_and_prerelease_dependencies() {
    for (change, expected, weight) in [
        (DependencyChange::Added, "dep.added", 0.3),
        (DependencyChange::Removed, "dep.removed", 0.25),
        (
            DependencyChange::Bumped(BumpKind::Prerelease),
            "dep.prerelease",
            0.4,
        ),
    ] {
        let signals = detect_manifest_signals(&with_dependencies(vec![dependency(change, false)]));
        assert_eq!(signals.len(), 1, "{expected}");
        assert_eq!(signals[0].rule_id, expected);
        assert_eq!(signals[0].weight, weight);
    }
}

#[test]
fn lock_drift_signal() {
    let lock_only = ChangeSet {
        files: vec![FileInput::new(
            path("package-lock.json"),
            FileChangeStatus::Modified,
        )],
        ..ChangeSet::default()
    };
    let signals = detect_manifest_signals(&lock_only);
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].rule_id, "dep.lock_drift");
    assert_eq!(signals[0].weight, 0.2);

    let with_manifest = ChangeSet {
        files: vec![
            FileInput::new(path("package-lock.json"), FileChangeStatus::Modified),
            FileInput::new(path("package.json"), FileChangeStatus::Modified),
        ],
        ..ChangeSet::default()
    };
    assert!(detect_manifest_signals(&with_manifest).is_empty());

    let flagged = detect_manifest_signals(&with_dependencies(vec![dependency(
        DependencyChange::Bumped(BumpKind::Patch),
        true,
    )]));
    assert_eq!(flagged[0].rule_id, "dep.lock_drift");
}

fn migration(destructive: bool, modified_existing: bool) -> SchemaInput {
    SchemaInput {
        kind: SchemaKind::Migration,
        path: path("db/migrations/0001_users.sql"),
        destructive,
        modified_existing,
        table: Some("users".to_owned()),
    }
}

#[test]
fn destructive_migration_floor_high() {
    let signals = detect_manifest_signals(&ChangeSet {
        schemas: vec![migration(true, false)],
        ..ChangeSet::default()
    });
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].rule_id, "migration.destructive");
    assert_eq!(signals[0].floor_level, Some(RiskLevel::High));
    assert_eq!(signals[0].category, RiskCategory::DatabaseMigrations);
}

#[test]
fn edited_existing_migration() {
    let signals = detect_manifest_signals(&ChangeSet {
        schemas: vec![migration(false, true)],
        ..ChangeSet::default()
    });
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].rule_id, "migration.edited_existing");
    assert_eq!(signals[0].weight, 0.7);
}

#[test]
fn new_migration_and_entity_change() {
    let signals = detect_manifest_signals(&ChangeSet {
        schemas: vec![
            migration(false, false),
            SchemaInput {
                kind: SchemaKind::Entity,
                path: path("src/users/user.entity.ts"),
                destructive: false,
                modified_existing: false,
                table: None,
            },
        ],
        ..ChangeSet::default()
    });
    let ids: Vec<&str> = signals.iter().map(|s| s.rule_id.as_str()).collect();
    assert!(ids.contains(&"migration.added"));
    assert!(ids.contains(&"schema.entity_changed"));
    assert_eq!(
        rule(&signals, "schema.entity_changed").unwrap().category,
        RiskCategory::SchemaChanges
    );
}
