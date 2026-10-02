//! Names of files that may hold secrets (INIT-002). Matching is on the path only: a file that
//! matches is never opened by any detector.

use std::sync::OnceLock;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use review_core::location::RepoPath;

/// Basename patterns, matched case-insensitively.
const BASENAME_PATTERNS: &[&str] = &[
    ".env",
    ".env.*",
    "*.env",
    ".envrc",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
    "*.keystore",
    "id_rsa*",
    "id_ed25519*",
    "*.ppk",
    "credentials.json",
    "*service-account*.json",
    "*-key.json",
    "gcs-key.json",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".git-credentials",
    "secrets.yml",
    "secrets.yaml",
    "*.tfstate",
    "*.tfstate.backup",
];

/// `.env.<suffix>` files that are templates: classified as source, and INIT-009 reads only
/// their variable names.
const ENV_TEMPLATE_SUFFIXES: &[&str] = &["example", "sample", "template", "dist", "defaults"];

/// Path suffixes (whole trailing segments) that are sensitive.
const PATH_SUFFIXES: &[&str] = &[".claude/settings.local.json"];

fn set() -> &'static GlobSet {
    static SET: OnceLock<GlobSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut builder = GlobSetBuilder::new();
        for pattern in BASENAME_PATTERNS {
            if let Ok(glob) = GlobBuilder::new(pattern).case_insensitive(true).build() {
                builder.add(glob);
            }
        }
        builder.build().unwrap_or_else(|_| GlobSet::empty())
    })
}

/// True for an `.env.<x>` style template (`.env.example`, `prod.env.sample`, ...).
pub fn is_env_template(basename: &str) -> bool {
    let lower = basename.to_ascii_lowercase();
    ENV_TEMPLATE_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(&format!(".{suffix}")))
        && (lower.starts_with(".env") || lower.contains(".env."))
}

/// Whether the path names a file that must never be opened.
pub fn is_sensitive(path: &RepoPath) -> bool {
    let text = path.as_str();
    let lower = text.to_ascii_lowercase();
    if PATH_SUFFIXES
        .iter()
        .any(|suffix| lower == *suffix || lower.ends_with(&format!("/{suffix}")))
    {
        return true;
    }
    let basename = text.rsplit('/').next().unwrap_or(text);
    if is_env_template(basename) {
        return false;
    }
    set().is_match(basename)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sensitive(path: &str) -> bool {
        is_sensitive(&RepoPath::new(path).unwrap())
    }

    #[test]
    fn matches_table() {
        for yes in [
            ".env",
            "apps/api/.env.production",
            "a/b/.ENV.local",
            "prod.env",
            ".envrc",
            "certs/server.pem",
            "tls/server.KEY",
            "id_rsa",
            "id_rsa.pub",
            "id_ed25519",
            "gcs-key.json",
            "my-service-account-prod.json",
            "credentials.json",
            "deploy-key.json",
            ".npmrc",
            ".netrc",
            "secrets.yaml",
            "secrets.yml",
            "infra/terraform.tfstate",
            "infra/terraform.tfstate.backup",
            ".claude/settings.local.json",
            "pkg/.claude/settings.local.json",
        ] {
            assert!(sensitive(yes), "{yes} should be sensitive");
        }
        for no in [
            ".env.example",
            ".env.sample",
            ".env.template",
            ".env.dist",
            ".env.defaults",
            "src/environment.ts",
            "README.md",
            "package.json",
            ".claude/settings.json",
            "keys.ts",
        ] {
            assert!(!sensitive(no), "{no} should not be sensitive");
        }
    }
}
