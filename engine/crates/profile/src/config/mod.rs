//! `.review/config.yaml`: schema, safe parsing, validation, normalization (POL-001) and the
//! snapshot-time sync that binds a config to a commit (POL-002).
//!
//! The one entry point is [`load_config`]. It never fails: an invalid file yields the defaults
//! plus the validation errors ([`ConfigStatus::Invalid`]), so reviews still run, and because the
//! effective config is the defaults, no suppression from a broken file is ever applied.

pub mod normalize;
pub mod schema;
pub mod sync;
pub mod validate;

use chrono::{NaiveDate, Utc};
use review_core::version::ConfigHash;
use schemars::schema::RootSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use schema::*;
pub use validate::{ConfigIssue, ConfigIssueKind, IssueSeverity};

/// Repository-relative location of the config file.
pub const CONFIG_PATH: &str = ".review/config.yaml";

/// The JSON Schema of [`ReviewConfigV1`] (published in contracts as `ReviewConfigV1`).
pub fn json_schema() -> RootSchema {
    schemars::gen::SchemaGenerator::default().into_root_schema_for::<ReviewConfigV1>()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigStatus {
    /// No file: the defaults apply.
    Missing,
    /// The file parsed and validated (warnings allowed).
    Valid,
    /// The file has at least one error: the defaults apply and the errors are surfaced.
    Invalid,
}

/// The effective configuration for one commit, with everything needed to store and explain it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    pub status: ConfigStatus,
    /// What the review uses: the parsed file when valid, the defaults otherwise.
    pub config: ReviewConfigV1,
    pub issues: Vec<ConfigIssue>,
    /// Defaults expanded, keys sorted.
    pub normalized: Value,
    pub config_hash: ConfigHash,
}

impl LoadedConfig {
    pub fn is_valid(&self) -> bool {
        self.status != ConfigStatus::Invalid
    }

    pub fn errors(&self) -> impl Iterator<Item = &ConfigIssue> {
        self.issues.iter().filter(|i| i.is_error())
    }

    /// Suppressions that may be applied. Always empty for an invalid file: the system never
    /// silently suppresses on a broken config.
    pub fn applicable_suppressions(&self) -> &[ConfigSuppression] {
        match self.status {
            ConfigStatus::Invalid => &[],
            _ => &self.config.suppressions,
        }
    }

    /// Hash of the graph-affecting keys only (INC-011 full-rebuild trigger).
    pub fn graph_inputs_hash(&self) -> [u8; 32] {
        normalize::graph_inputs_hash(&self.config)
    }

    /// `review doctor` lines, one per issue, each naming its key path.
    pub fn doctor_report(&self) -> Vec<String> {
        self.issues.iter().map(ConfigIssue::render).collect()
    }
}

/// Loads a config from its raw bytes (`None` when the file is absent), using today's date for
/// suppression expiry.
pub fn load_config(raw: Option<&[u8]>) -> LoadedConfig {
    load_config_at(raw, Utc::now().date_naive())
}

/// [`load_config`] with an explicit "today", for deterministic tests.
pub fn load_config_at(raw: Option<&[u8]>, today: NaiveDate) -> LoadedConfig {
    let defaults = ReviewConfigV1::default();
    let Some(raw) = raw else {
        return LoadedConfig {
            status: ConfigStatus::Missing,
            normalized: normalize::normalized(&defaults),
            config_hash: normalize::config_hash(&defaults),
            config: defaults,
            issues: Vec::new(),
        };
    };

    let (parsed, issues) = parse_and_validate(raw, today);
    let loaded = match parsed {
        Some(config) if !issues.iter().any(ConfigIssue::is_error) => LoadedConfig {
            status: ConfigStatus::Valid,
            normalized: normalize::normalized(&config),
            config_hash: normalize::config_hash(&config),
            config,
            issues,
        },
        _ => LoadedConfig {
            status: ConfigStatus::Invalid,
            normalized: normalize::normalized(&defaults),
            config_hash: normalize::invalid_config_hash(&defaults, raw),
            config: defaults,
            issues,
        },
    };
    for issue in loaded.errors() {
        crate::metrics::config_validation_error(issue.kind.as_str());
    }
    loaded
}

fn parse_and_validate(raw: &[u8], today: NaiveDate) -> (Option<ReviewConfigV1>, Vec<ConfigIssue>) {
    let value = match validate::parse_yaml_safely(raw) {
        Ok(value) => value,
        Err(issue) => return (None, vec![issue]),
    };
    if !value.is_object() {
        return (
            None,
            vec![ConfigIssue::error(
                ConfigIssueKind::InvalidValue,
                "",
                "the config must be a mapping",
            )],
        );
    }
    let mut issues = validate::unknown_keys(&value, &json_schema());
    if !issues.is_empty() {
        return (None, issues);
    }
    let config: ReviewConfigV1 = match serde_json::from_value(value) {
        Ok(config) => config,
        Err(e) => {
            issues.push(ConfigIssue::error(
                ConfigIssueKind::InvalidValue,
                "",
                e.to_string(),
            ));
            return (None, issues);
        }
    };
    issues.extend(validate::validate_semantics(&config, today));
    (Some(config), issues)
}
