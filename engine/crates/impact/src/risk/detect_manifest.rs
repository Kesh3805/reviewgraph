//! Manifest, lockfile, migration and entity signals (RISK-002).

use review_core::location::RepoPath;

use super::new_signal;
use super::signal::{dedup_signals, RiskSignal, SignalEvidence, SignalSubject};
use crate::input::{BumpKind, ChangeSet, DependencyChange, SchemaKind};

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "Cargo.lock",
];
const MANIFESTS: &[&str] = &["package.json", "Cargo.toml"];

fn file_name(path: &RepoPath) -> &str {
    path.as_str().rsplit('/').next().unwrap_or("")
}

fn file_evidence(path: &RepoPath, detail: String) -> Vec<SignalEvidence> {
    vec![SignalEvidence {
        file: Some(path.clone()),
        detail,
        ..SignalEvidence::default()
    }]
}

/// Dependency, lockfile and schema signals.
pub fn detect_manifest_signals(changes: &ChangeSet) -> Vec<RiskSignal> {
    let mut out = Vec::new();

    for dependency in &changes.dependencies {
        let rule_id = if dependency.lockfile_only {
            "dep.lock_drift"
        } else {
            match dependency.change {
                DependencyChange::Added | DependencyChange::SourceChanged => "dep.added",
                DependencyChange::Removed => "dep.removed",
                DependencyChange::Bumped(BumpKind::Major | BumpKind::Downgrade) => "dep.major_bump",
                DependencyChange::Bumped(BumpKind::Prerelease) => "dep.prerelease",
                DependencyChange::Bumped(BumpKind::Minor | BumpKind::Patch) => continue,
            }
        };
        out.extend(new_signal(
            rule_id,
            SignalSubject::File(dependency.manifest.clone()),
            1.0,
            file_evidence(
                &dependency.manifest,
                format!("dependency {}", dependency.name),
            ),
        ));
    }

    // A lockfile that changed while no manifest did.
    let manifest_changed = changes
        .files
        .iter()
        .any(|file| MANIFESTS.contains(&file_name(&file.path)));
    if !manifest_changed {
        for file in &changes.files {
            if LOCKFILES.contains(&file_name(&file.path)) {
                out.extend(new_signal(
                    "dep.lock_drift",
                    SignalSubject::File(file.path.clone()),
                    1.0,
                    file_evidence(
                        &file.path,
                        "lockfile changed without a manifest change".to_owned(),
                    ),
                ));
            }
        }
    }

    for schema in &changes.schemas {
        let table = schema
            .table
            .as_deref()
            .map_or(String::new(), |table| format!(" (table {table})"));
        let mut rule_ids: Vec<&str> = Vec::new();
        match schema.kind {
            SchemaKind::Migration => {
                if schema.destructive {
                    rule_ids.push("migration.destructive");
                }
                if schema.modified_existing {
                    rule_ids.push("migration.edited_existing");
                }
                if rule_ids.is_empty() {
                    rule_ids.push("migration.added");
                }
            }
            SchemaKind::Entity => rule_ids.push("schema.entity_changed"),
        }
        for rule_id in rule_ids {
            out.extend(new_signal(
                rule_id,
                SignalSubject::File(schema.path.clone()),
                1.0,
                file_evidence(&schema.path, format!("schema change{table}")),
            ));
        }
    }
    dedup_signals(out)
}
