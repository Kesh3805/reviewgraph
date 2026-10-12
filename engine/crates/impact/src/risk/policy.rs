//! Risk policy (RISK-002): the configuration the risk engine reads, as plain data.
//!
//! `impact` does not read `.review/config.yaml`; the pipeline maps POL-001's `risk.paths`,
//! `risk.authorization_symbols`, `risk.payment_paths` and the generated globs into a
//! [`RiskPolicy`]. Globs are compiled once here, with `literal_separator` so `*` stays inside one
//! path segment and `**` crosses segments.

use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};

use super::level::RiskLevel;
use super::signal::RiskCategory;
use crate::error::Error;

/// Default payment globs (`risk.payment_paths`).
pub const DEFAULT_PAYMENT_PATHS: [&str; 2] = ["**/payment*/**", "**/billing/**"];

fn glob(pattern: &str) -> Result<Glob, Error> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| Error::InvalidConfig(format!("invalid glob {pattern:?}: {e}")))
}

fn glob_set(patterns: &[String]) -> Result<GlobSet, Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(glob(pattern)?);
    }
    builder
        .build()
        .map_err(|e| Error::InvalidConfig(format!("invalid glob set: {e}")))
}

/// Infers a category from the keywords of a risk-path glob: `auth` → authentication,
/// `permission|policy|acl` → permissions, `payment|billing` → payments, `migration` →
/// database migrations, `crypto|security` → cryptography, `secret` → secrets, otherwise
/// public API contracts.
pub fn infer_category(glob: &str) -> RiskCategory {
    let lower = glob.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    if has(&["auth"]) {
        RiskCategory::Authentication
    } else if has(&["permission", "policy", "acl"]) {
        RiskCategory::Permissions
    } else if has(&["payment", "billing"]) {
        RiskCategory::Payments
    } else if has(&["migration"]) {
        RiskCategory::DatabaseMigrations
    } else if has(&["crypto", "security"]) {
        RiskCategory::Cryptography
    } else if has(&["secret"]) {
        RiskCategory::Secrets
    } else {
        RiskCategory::PublicApiContracts
    }
}

/// One configured `risk.paths` entry.
#[derive(Debug, Clone)]
pub struct PathRule {
    pub glob: String,
    pub level: RiskLevel,
    /// The configured category, or the one inferred from the glob.
    pub category: RiskCategory,
    matcher: GlobMatcher,
}

impl PathRule {
    pub fn new(glob_text: &str, level: RiskLevel, category: Option<RiskCategory>) -> Result<Self, Error> {
        Ok(Self {
            glob: glob_text.to_owned(),
            level,
            category: category.unwrap_or_else(|| infer_category(glob_text)),
            matcher: glob(glob_text)?.compile_matcher(),
        })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.matcher.is_match(path)
    }
}

/// The risk configuration of one repository.
#[derive(Debug, Clone)]
pub struct RiskPolicy {
    /// Ordered; the first match wins.
    pub paths: Vec<PathRule>,
    /// Extra authorization symbol names (identifiers or callee names).
    pub authorization_symbols: Vec<String>,
    generated_patterns: Vec<String>,
    generated: GlobSet,
    payment_patterns: Vec<String>,
    payments: GlobSet,
}

impl Default for RiskPolicy {
    fn default() -> Self {
        let payment_patterns: Vec<String> =
            DEFAULT_PAYMENT_PATHS.iter().map(|p| (*p).to_owned()).collect();
        Self {
            paths: Vec::new(),
            authorization_symbols: Vec::new(),
            generated_patterns: Vec::new(),
            generated: GlobSet::empty(),
            payments: glob_set(&payment_patterns).unwrap_or_else(|_| GlobSet::empty()),
            payment_patterns,
        }
    }
}

impl RiskPolicy {
    /// Builds a policy; invalid globs are rejected (POL-001 rejects them at load already).
    pub fn new(
        paths: Vec<(String, RiskLevel, Option<RiskCategory>)>,
        authorization_symbols: Vec<String>,
        generated_globs: Vec<String>,
        payment_paths: Option<Vec<String>>,
    ) -> Result<Self, Error> {
        let rules = paths
            .iter()
            .map(|(pattern, level, category)| PathRule::new(pattern, *level, *category))
            .collect::<Result<Vec<_>, _>>()?;
        let payment_patterns = payment_paths.unwrap_or_else(|| {
            DEFAULT_PAYMENT_PATHS.iter().map(|p| (*p).to_owned()).collect()
        });
        Ok(Self {
            paths: rules,
            authorization_symbols,
            generated: glob_set(&generated_globs)?,
            generated_patterns: generated_globs,
            payments: glob_set(&payment_patterns)?,
            payment_patterns,
        })
    }

    /// The first configured rule matching `path`.
    pub fn path_rule(&self, path: &str) -> Option<&PathRule> {
        self.paths.iter().find(|rule| rule.matches(path))
    }

    pub fn is_payment_path(&self, path: &str) -> bool {
        self.payments.is_match(path)
    }

    pub fn is_generated(&self, path: &str) -> bool {
        self.generated.is_match(path)
    }

    /// A stable hash of the policy, part of the risk input hash.
    pub fn hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        for rule in &self.paths {
            hasher.update(b"path\0");
            hasher.update(rule.glob.as_bytes());
            hasher.update(rule.level.as_str().as_bytes());
            hasher.update(rule.category.as_str().as_bytes());
        }
        for (tag, items) in [
            ("authz", &self.authorization_symbols),
            ("generated", &self.generated_patterns),
            ("payments", &self.payment_patterns),
        ] {
            hasher.update(tag.as_bytes());
            for item in items {
                hasher.update(&(item.len() as u64).to_le_bytes());
                hasher.update(item.as_bytes());
            }
        }
        hasher.finalize().to_hex().to_string()
    }
}
