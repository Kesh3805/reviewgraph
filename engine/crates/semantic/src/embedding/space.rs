//! Embedding space identity (SEM-001, ADR-008).
//!
//! Vectors from different spaces are never comparable, so the space is part of every point id
//! and of the collection name.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Embedding provider family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderName {
    Openai,
    Voyage,
    /// Deterministic offline feature-hashing provider (SEM-002).
    Hash,
}

impl ProviderName {
    pub const ALL: [ProviderName; 3] = [Self::Openai, Self::Voyage, Self::Hash];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Voyage => "voyage",
            Self::Hash => "hash",
        }
    }

    /// Whether the provider sends text to a third party.
    pub const fn is_external(self) -> bool {
        !matches!(self, Self::Hash)
    }
}

impl fmt::Display for ProviderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| Error::Config(format!("unknown embedding provider {s:?}")))
    }
}

/// Provider, model, dimensionality and our re-embed epoch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EmbeddingSpace {
    pub provider: ProviderName,
    pub model: String,
    pub dims: u16,
    /// Re-embed epoch: bumped to force a parallel collection for the same model.
    pub version: u16,
}

/// Lowercases and replaces every character outside `[a-z0-9]` with `_`, so free text can never
/// inject URL or path syntax into a collection name.
pub(crate) fn sanitize(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl EmbeddingSpace {
    pub fn new(
        provider: ProviderName,
        model: impl Into<String>,
        dims: u16,
        version: u16,
    ) -> Result<Self, Error> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(Error::Config("embedding model must be non-empty".into()));
        }
        if dims == 0 {
            return Err(Error::Config(
                "embedding dims must be greater than 0".into(),
            ));
        }
        if version == 0 {
            return Err(Error::Config("embedding space version starts at 1".into()));
        }
        Ok(Self {
            provider,
            model,
            dims,
            version,
        })
    }

    /// `{provider}-{model}-{dims}` with the model sanitized to `[a-z0-9_]`.
    pub fn id(&self) -> String {
        format!(
            "{}-{}-{}",
            self.provider.as_str(),
            sanitize(&self.model),
            self.dims
        )
    }

    /// `rg_{provider}_{model}_{dims}_v{version}` (ADR-008).
    pub fn collection_name(&self) -> String {
        format!(
            "rg_{}_{}_{}_v{}",
            self.provider.as_str(),
            sanitize(&self.model),
            self.dims,
            self.version
        )
    }

    /// The `review-core` space value recorded on model-derived artifacts (ADR-015).
    pub fn core(&self) -> Result<review_core::version::EmbeddingSpace, Error> {
        Ok(review_core::version::EmbeddingSpace::new(
            self.provider.as_str(),
            self.model.clone(),
            u32::from(self.dims),
        )?)
    }
}

impl fmt::Display for EmbeddingSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-v{}", self.id(), self.version)
    }
}
