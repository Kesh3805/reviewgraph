//! The risk policy (RISK-002): the configured risk paths and the symbol and path lists the
//! detectors consult. The pipeline maps the POL-001 configuration (`risk.paths`,
//! `risk.authorization_symbols`, `risk.payment_paths`) into a [`RiskPolicy`]; this crate never
//! reads configuration files.

use std::sync::OnceLock;

use globset::{Glob, GlobMatcher, GlobSet, GlobSetBuilder};
use review_core::location::RepoPath;

use super::level::RiskLevel;
use super::signal::RiskCategory;
use crate::error::{Error, Result};

/// Glob patterns of the default payment paths.
pub const DEFAULT_PAYMENT_GLOBS: &[&str] = &["**/payment*/**", "**/billing/**"];

/// The default path rules used when the configuration lists no risk paths: rule id and globs.
const DEFAULT_PATH_RULES: &[(&str, &[&str])] = &[
    ("path.auth", &["**/auth/**", "**/security/**"]),
    ("path.migrations", &["**/migrations/**"]),
    (
        "path.infra",
        &[
            "**/Dockerfile",
            "**/Dockerfile.*",
            ".*/workflows/*.yml",
            ".*/workflows/*.yaml",
        ],
    ),
];

/// The weight of a configured path rule by level.
pub const fn level_weight(level: RiskLevel) -> f32 {
    match level {
        RiskLevel::Critical => 0.8,
        RiskLevel::High => 0.6,
        RiskLevel::Medium => 0.4,
        RiskLevel::Low => 0.1,
    }
}

/// The category a configured glob implies, from keywords in the glob text.
pub fn infer_category(glob: &str) -> RiskCategory {
    let glob = glob.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| glob.contains(word));
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

fn set_of(globs: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        let compiled = Glob::new(glob)
            .map_err(|e| Error::InvalidConfig(format!("invalid glob {glob:?}: {e}")))?;
        builder.add(compiled);
    }
    builder
        .build()
        .map_err(|e| Error::InvalidConfig(format!("invalid glob set: {e}")))
}

/// One configured `risk.paths` entry.
#[derive(Debug, Clone)]
pub struct PathRule {
    pub glob: String,
    pub level: RiskLevel,
    /// An explicit `category:` of the configuration entry; inferred from the glob when absent.
    pub category: Option<RiskCategory>,
    matcher: GlobMatcher,
}

impl PathRule {
    /// Compiles `glob`; an invalid glob is a configuration error.
    pub fn new(glob: &str, level: RiskLevel, category: Option<RiskCategory>) -> Result<Self> {
        let matcher = Glob::new(glob)
            .map_err(|e| Error::InvalidConfig(format!("invalid risk path {glob:?}: {e}")))?
            .compile_matcher();
        Ok(Self {
            glob: glob.to_owned(),
            level,
            category,
            matcher,
        })
    }

    pub fn matches(&self, path: &RepoPath) -> bool {
        self.matcher.is_match(path.as_str())
    }

    /// The explicit category, or the one inferred from the glob.
    pub fn effective_category(&self) -> RiskCategory {
        self.category.unwrap_or_else(|| infer_category(&self.glob))
    }
}

/// A built-in path rule.
#[derive(Debug, Clone)]
pub(crate) struct DefaultPathRule {
    pub rule_id: &'static str,
    pub globs: GlobSet,
}

/// The built-in path rules, compiled once.
pub(crate) fn default_path_rules() -> &'static [DefaultPathRule] {
    static RULES: OnceLock<Vec<DefaultPathRule>> = OnceLock::new();
    RULES.get_or_init(|| {
        DEFAULT_PATH_RULES
            .iter()
            .filter_map(|&(rule_id, globs)| {
                set_of(globs)
                    .ok()
                    .map(|globs| DefaultPathRule { rule_id, globs })
            })
            .collect()
    })
}

/// Policy inputs of the risk detectors.
#[derive(Debug, Clone)]
pub struct RiskPolicy {
    /// Ordered; the first matching rule wins.
    pub paths: Vec<PathRule>,
    /// Names of authorization entry points (`risk.authorization_symbols`); a removed call to
    /// one of them is an authorization call removal even when no change class says so.
    pub authorization_symbols: Vec<String>,
    /// Extra globs treated as generated (no path or secret signals).
    pub generated_globs: GlobSet,
    pub payment_paths: GlobSet,
    generated_sources: Vec<String>,
    payment_sources: Vec<String>,
}

impl Default for RiskPolicy {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            authorization_symbols: Vec::new(),
            generated_globs: GlobSet::empty(),
            payment_paths: set_of(DEFAULT_PAYMENT_GLOBS).unwrap_or_else(|_| GlobSet::empty()),
            generated_sources: Vec::new(),
            payment_sources: DEFAULT_PAYMENT_GLOBS
                .iter()
                .map(|glob| (*glob).to_owned())
                .collect(),
        }
    }
}

impl RiskPolicy {
    pub fn with_paths(mut self, paths: Vec<PathRule>) -> Self {
        self.paths = paths;
        self
    }

    pub fn with_authorization_symbols(mut self, symbols: Vec<String>) -> Self {
        self.authorization_symbols = symbols;
        self
    }

    pub fn with_generated_globs(mut self, globs: &[&str]) -> Result<Self> {
        self.generated_globs = set_of(globs)?;
        self.generated_sources = globs.iter().map(|glob| (*glob).to_owned()).collect();
        Ok(self)
    }

    pub fn with_payment_paths(mut self, globs: &[&str]) -> Result<Self> {
        self.payment_paths = set_of(globs)?;
        self.payment_sources = globs.iter().map(|glob| (*glob).to_owned()).collect();
        Ok(self)
    }

    /// Whether `callee` (a possibly dotted callee name) is a configured authorization symbol.
    pub fn is_authorization_symbol(&self, callee: &str) -> bool {
        let last = callee.rsplit('.').next().unwrap_or(callee);
        self.authorization_symbols
            .iter()
            .any(|name| name == callee || name == last)
    }

    /// `blake3` over everything the policy contributes to the risk input hash, as 64 hex
    /// characters.
    pub fn policy_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        let mut put = |part: &str| {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        };
        for rule in &self.paths {
            put(&rule.glob);
            put(rule.level.as_str());
            put(rule.effective_category().as_str());
        }
        put("|authorization");
        for name in &self.authorization_symbols {
            put(name);
        }
        put("|generated");
        for glob in &self.generated_sources {
            put(glob);
        }
        put("|payments");
        for glob in &self.payment_sources {
            put(glob);
        }
        hasher.finalize().to_hex().to_string()
    }
}
