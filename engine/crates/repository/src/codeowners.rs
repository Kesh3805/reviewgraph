//! CODEOWNERS parsing (INIT-010).

use std::sync::OnceLock;

use regex::Regex;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodeownersRule {
    pub pattern: String,
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodeownersFact {
    pub path: RepoPath,
    pub rules: Vec<CodeownersRule>,
    /// 1-based line numbers that are neither comments nor `pattern owner+` lines.
    pub invalid_lines: Vec<u32>,
}

/// Locations in GitHub's lookup order.
pub const CODEOWNERS_PATHS: &[&str] = &[".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];

fn owner_regexes() -> &'static Option<(Regex, Regex)> {
    static RE: OnceLock<Option<(Regex, Regex)>> = OnceLock::new();
    RE.get_or_init(|| {
        Some((
            Regex::new(r"^@[\w-]+(/[\w-]+)?$").ok()?,
            Regex::new(r"^[^@\s]+@[^@\s]+\.[^@\s]+$").ok()?,
        ))
    })
}

/// Parses CODEOWNERS text. Owners are stored exactly as written.
pub fn parse_codeowners(path: RepoPath, text: &str) -> CodeownersFact {
    let mut rules = Vec::new();
    let mut invalid = Vec::new();
    let regexes = owner_regexes();
    for (index, raw) in text.lines().enumerate() {
        let line_no = (index + 1) as u32;
        // A `#` starts a comment, unless it is escaped inside a pattern.
        let line = raw.split(" #").next().unwrap_or(raw).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(pattern) = parts.next() else {
            continue;
        };
        let owners: Vec<String> = parts.map(str::to_owned).collect();
        let valid = !owners.is_empty()
            && regexes.as_ref().is_some_and(|(handle, email)| {
                owners
                    .iter()
                    .all(|o| handle.is_match(o) || email.is_match(o))
            });
        if valid {
            rules.push(CodeownersRule {
                pattern: pattern.to_owned(),
                owners,
            });
        } else {
            invalid.push(line_no);
        }
    }
    CodeownersFact {
        path,
        rules,
        invalid_lines: invalid,
    }
}
