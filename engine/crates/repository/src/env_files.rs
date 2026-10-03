//! Environment files: names only, never values (INIT-009).
//!
//! Real env files are classified `Sensitive` by the walker and are never opened. Templates are
//! read through the bounded reader and only the variable names are kept; the value is dropped
//! inside the matching loop.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{InitWarning, WarningSeverity};
use crate::read::BoundedReader;
use crate::sensitive::is_env_template;
use crate::walk::{FileClass, FileInventory};

const MAX_TEMPLATE_BYTES: usize = 64 * 1024;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EnvFileKind {
    Template,
    Real,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvFileFact {
    pub path: RepoPath,
    pub kind: EnvFileKind,
    pub tracked_in_git: bool,
    /// Template files only; always empty for `Real`.
    pub variable_names: Vec<String>,
}

fn is_env_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env" || lower.starts_with(".env.") || lower.ends_with(".env") || lower == ".envrc"
}

fn key_regex() -> &'static Option<Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=").ok())
}

/// Env files in the inventory. `tracked` holds the repo paths present in the git index (see
/// [`crate::git::tracked_among`]).
pub fn detect_env_files(
    inv: &FileInventory,
    reader: &BoundedReader,
    tracked: &BTreeSet<RepoPath>,
) -> (Vec<EnvFileFact>, Vec<InitWarning>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for entry in &inv.entries {
        let name = entry.path.as_str().rsplit('/').next().unwrap_or("");
        if !is_env_file_name(name) {
            continue;
        }
        let tracked_in_git = tracked.contains(&entry.path);
        if entry.class == FileClass::Sensitive {
            if tracked_in_git {
                warnings.push(
                    InitWarning::new(
                        "env_file_committed",
                        Some(entry.path.clone()),
                        "a real environment file is tracked in git; its content was not inspected",
                    )
                    .with_severity(WarningSeverity::High),
                );
            }
            out.push(EnvFileFact {
                path: entry.path.clone(),
                kind: EnvFileKind::Real,
                tracked_in_git,
                variable_names: Vec::new(),
            });
        } else if is_env_template(name) {
            let mut names = BTreeSet::new();
            if let (Ok(text), Some(re)) = (reader.read_text(entry, MAX_TEMPLATE_BYTES), key_regex())
            {
                for line in text.lines() {
                    if let Some(key) = re.captures(line).and_then(|c| c.get(1)) {
                        names.insert(key.as_str().to_owned());
                    }
                }
            }
            out.push(EnvFileFact {
                path: entry.path.clone(),
                kind: EnvFileKind::Template,
                tracked_in_git,
                variable_names: names.into_iter().collect(),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    warnings.sort();
    (out, warnings)
}
