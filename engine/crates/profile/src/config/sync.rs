//! Config sync at snapshot time (POL-002).
//!
//! The policy that governs a review is the one **at the reviewed commit's base**: the indexer
//! reads `.review/config.yaml` from the snapshot tree ([`sync_config`]), and a pull request is
//! reviewed under its base policy ([`bind_pull_request_policy`]). A head that edits rules or
//! suppressions is reported as the risk signal [`REVIEW_POLICY_CHANGED`], never applied, so a PR
//! can neither weaken nor disable its own review.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use review_core::version::ConfigHash;
use serde::{Deserialize, Serialize};

use super::{
    load_config, load_config_at, ConfigIssue, ConfigIssueKind, ConfigStatus, LoadedConfig,
};
use super::{normalize, ReviewConfigV1, CONFIG_PATH};

/// The risk-signal name a policy-changing pull request raises (RISK-001 vocabulary).
pub const REVIEW_POLICY_CHANGED: &str = "review_policy_changed";

/// One file read from a snapshot tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeBlob {
    pub bytes: Vec<u8>,
    /// The git blob id, when the tree is a git tree.
    pub blob_sha: Option<String>,
}

/// Read access to the files of one snapshot (a git tree, a working directory, a test map).
pub trait SnapshotTree {
    /// `Ok(None)` when the path does not exist in the snapshot.
    fn read_file(&self, path: &str) -> Result<Option<TreeBlob>, String>;
}

/// A snapshot backed by a working directory.
#[derive(Debug, Clone)]
pub struct WorkingTree {
    root: PathBuf,
}

impl WorkingTree {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl SnapshotTree for WorkingTree {
    fn read_file(&self, path: &str) -> Result<Option<TreeBlob>, String> {
        match std::fs::read(self.root.join(path)) {
            Ok(bytes) => Ok(Some(TreeBlob {
                bytes,
                blob_sha: None,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// A snapshot held in memory: path → content.
#[derive(Debug, Clone, Default)]
pub struct MemoryTree {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl MemoryTree {
    pub fn with(mut self, path: &str, content: impl Into<Vec<u8>>) -> Self {
        self.files.insert(path.to_owned(), content.into());
        self
    }
}

impl SnapshotTree for MemoryTree {
    fn read_file(&self, path: &str) -> Result<Option<TreeBlob>, String> {
        Ok(self.files.get(path).map(|bytes| TreeBlob {
            bytes: bytes.clone(),
            blob_sha: None,
        }))
    }
}

/// The config bound to one snapshot: what `repository_configs` stores.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncedConfig {
    pub loaded: LoadedConfig,
    /// `.review/config.yaml` when the file exists in the snapshot (`snapshots.config_source_path`).
    pub source_path: Option<String>,
    pub raw_blob_sha: Option<String>,
}

impl SyncedConfig {
    pub fn config_hash(&self) -> ConfigHash {
        self.loaded.config_hash
    }
}

/// Reads and validates the config of one snapshot.
pub fn sync_config(tree: &dyn SnapshotTree) -> SyncedConfig {
    sync_with(tree, load_config)
}

/// [`sync_config`] with an explicit "today" for suppression expiry.
pub fn sync_config_at(tree: &dyn SnapshotTree, today: chrono::NaiveDate) -> SyncedConfig {
    sync_with(tree, |raw| load_config_at(raw, today))
}

fn sync_with(
    tree: &dyn SnapshotTree,
    load: impl Fn(Option<&[u8]>) -> LoadedConfig,
) -> SyncedConfig {
    let span = tracing::info_span!("config_sync", config_hash = tracing::field::Empty);
    let _guard = span.enter();
    let synced = match tree.read_file(CONFIG_PATH) {
        Ok(None) => SyncedConfig {
            loaded: load(None),
            source_path: None,
            raw_blob_sha: None,
        },
        Ok(Some(blob)) => SyncedConfig {
            loaded: load(Some(&blob.bytes)),
            source_path: Some(CONFIG_PATH.to_owned()),
            raw_blob_sha: blob.blob_sha,
        },
        Err(reason) => {
            // Unreadable is not absent: fail closed to the defaults with a visible error.
            let defaults = ReviewConfigV1::default();
            let loaded = LoadedConfig {
                status: ConfigStatus::Invalid,
                normalized: normalize::normalized(&defaults),
                config_hash: normalize::invalid_config_hash(&defaults, reason.as_bytes()),
                config: defaults,
                issues: vec![ConfigIssue::error(
                    ConfigIssueKind::Parse,
                    "",
                    format!("{CONFIG_PATH} could not be read: {reason}"),
                )],
            };
            SyncedConfig {
                loaded,
                source_path: Some(CONFIG_PATH.to_owned()),
                raw_blob_sha: None,
            }
        }
    };
    span.record("config_hash", synced.loaded.config_hash.to_string());
    synced
}

/// Whether moving from `previous` to `next` invalidates the graph (INC-011): only ignore and
/// generated-code globs and index tolerance count. Rule, suppression and reviewer changes never
/// rebuild the graph.
pub fn requires_graph_rebuild(previous: &LoadedConfig, next: &LoadedConfig) -> bool {
    previous.graph_inputs_hash() != next.graph_inputs_hash()
}

/// How a pull request's head config differs from its base, in the parts that govern review.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyChange {
    /// Top-level policy sections that differ (`rules`, `suppressions`, `review.reviewers`, ...).
    pub changed_sections: Vec<String>,
    pub suppressions_added: Vec<String>,
    pub suppressions_removed: Vec<String>,
    /// Forbidden-dependency rule ids added or removed.
    pub rules_added: Vec<String>,
    pub rules_removed: Vec<String>,
    /// The head file does not validate.
    pub head_invalid: bool,
}

impl PolicyChange {
    /// The summary line the PR comment carries.
    pub fn summary_line(&self) -> String {
        let mut parts = Vec::new();
        if !self.changed_sections.is_empty() {
            parts.push(format!("changed {}", self.changed_sections.join(", ")));
        }
        if !self.suppressions_added.is_empty() {
            parts.push(format!(
                "adds suppressions {}",
                self.suppressions_added.join(", ")
            ));
        }
        if !self.rules_removed.is_empty() {
            parts.push(format!("removes rules {}", self.rules_removed.join(", ")));
        }
        if self.head_invalid {
            parts.push("the new config does not validate".to_owned());
        }
        format!(
            "This PR changes review policy ({}). It was reviewed under the base branch policy; \
             the change takes effect after merge.",
            parts.join("; ")
        )
    }
}

/// The policy a pull request is reviewed under.
#[derive(Debug, Clone, PartialEq)]
pub struct PullRequestPolicy {
    /// Always the base config.
    pub effective: LoadedConfig,
    pub head_config_hash: ConfigHash,
    /// `Some` when the head edits review policy: raise [`REVIEW_POLICY_CHANGED`].
    pub change: Option<PolicyChange>,
}

impl PullRequestPolicy {
    pub fn risk_signals(&self) -> Vec<&'static str> {
        if self.change.is_some() {
            vec![REVIEW_POLICY_CHANGED]
        } else {
            Vec::new()
        }
    }
}

/// Binds a pull request to its base policy and diffs the head against it.
pub fn bind_pull_request_policy(base: &LoadedConfig, head: &LoadedConfig) -> PullRequestPolicy {
    let change = policy_change(base, head);
    if change.is_some() {
        crate::metrics::review_policy_changed();
    }
    PullRequestPolicy {
        effective: base.clone(),
        head_config_hash: head.config_hash,
        change,
    }
}

fn policy_change(base: &LoadedConfig, head: &LoadedConfig) -> Option<PolicyChange> {
    if base.config_hash == head.config_hash {
        return None;
    }
    let (b, h) = (&base.config, &head.config);
    let mut change = PolicyChange {
        head_invalid: head.status == ConfigStatus::Invalid,
        ..PolicyChange::default()
    };
    let sections: [(&str, bool); 8] = [
        ("rules", b.rules != h.rules),
        ("suppressions", b.suppressions != h.suppressions),
        ("architecture", b.architecture != h.architecture),
        ("conventions", b.conventions != h.conventions),
        (
            "knowledge_sources",
            b.knowledge_sources != h.knowledge_sources,
        ),
        ("review.reviewers", b.review.reviewers != h.review.reviewers),
        (
            "review.confidence",
            b.review.confidence != h.review.confidence,
        ),
        ("review.risk", b.review.risk != h.review.risk),
    ];
    change.changed_sections = sections
        .iter()
        .filter(|(_, differs)| *differs)
        .map(|(name, _)| (*name).to_owned())
        .collect();
    let ids = |c: &ReviewConfigV1| -> BTreeSet<String> {
        c.suppressions.iter().map(|s| s.id.clone()).collect()
    };
    let (bs, hs) = (ids(b), ids(h));
    change.suppressions_added = hs.difference(&bs).cloned().collect();
    change.suppressions_removed = bs.difference(&hs).cloned().collect();
    let rule_ids = |c: &ReviewConfigV1| -> BTreeSet<String> {
        c.rules
            .forbidden_dependencies
            .iter()
            .map(|r| r.id.clone())
            .collect()
    };
    let (br, hr) = (rule_ids(b), rule_ids(h));
    change.rules_added = hr.difference(&br).cloned().collect();
    change.rules_removed = br.difference(&hr).cloned().collect();
    if change.changed_sections.is_empty() && !change.head_invalid {
        // Only non-policy keys (budgets, publish, ignore globs) changed.
        return None;
    }
    Some(change)
}
