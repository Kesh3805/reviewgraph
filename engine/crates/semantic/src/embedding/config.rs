//! Provider selection (SEM-002): `semantic.embedding.{provider,model,dims,concurrency}`.

use std::sync::Arc;

use telemetry::Secret;

use super::{
    standard, EmbeddingProvider, HashProvider, OpenAiProvider, ProviderName, VoyageProvider,
};
use crate::error::Error;

/// Embedding settings. `hash` is the default for development and tests; `voyage` is recommended
/// in production.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub provider: ProviderName,
    pub model: Option<String>,
    pub dims: Option<u16>,
    /// Parallel requests per provider instance.
    pub concurrency: usize,
    /// Re-embed epoch of the space.
    pub version: u16,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: ProviderName::Hash,
            model: None,
            dims: None,
            concurrency: 4,
            version: 1,
        }
    }
}

/// Repository privacy class relevant to embeddings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Privacy {
    #[default]
    Standard,
    /// No repository text may leave the deployment: forces the `hash` provider.
    NoExternal,
}

impl EmbeddingConfig {
    /// Reads `SEMANTIC_EMBEDDING_{PROVIDER,MODEL,DIMS,CONCURRENCY,VERSION}` through `lookup`.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let mut cfg = Self::default();
        if let Some(p) = lookup("SEMANTIC_EMBEDDING_PROVIDER") {
            cfg.provider = p.trim().parse()?;
        }
        cfg.model = lookup("SEMANTIC_EMBEDDING_MODEL").filter(|m| !m.trim().is_empty());
        let num = |key: &str| -> Result<Option<u64>, Error> {
            lookup(key)
                .map(|v| {
                    v.trim()
                        .parse::<u64>()
                        .map_err(|e| Error::Config(format!("{key}: {e}")))
                })
                .transpose()
        };
        if let Some(d) = num("SEMANTIC_EMBEDDING_DIMS")? {
            cfg.dims =
                Some(u16::try_from(d).map_err(|_| {
                    Error::Config(format!("SEMANTIC_EMBEDDING_DIMS {d} too large"))
                })?);
        }
        if let Some(c) = num("SEMANTIC_EMBEDDING_CONCURRENCY")? {
            cfg.concurrency = usize::try_from(c).unwrap_or(usize::MAX).max(1);
        }
        if let Some(v) = num("SEMANTIC_EMBEDDING_VERSION")? {
            cfg.version = u16::try_from(v)
                .map_err(|_| Error::Config(format!("SEMANTIC_EMBEDDING_VERSION {v} too large")))?;
        }
        Ok(cfg)
    }

    /// The provider actually used: `no_external` forces `hash` (fail-safe, logged).
    pub fn effective_provider(&self, privacy: Privacy) -> ProviderName {
        if privacy == Privacy::NoExternal && self.provider.is_external() {
            tracing::warn!(
                configured = self.provider.as_str(),
                "privacy no_external forces the hash embedding provider"
            );
            return ProviderName::Hash;
        }
        self.provider
    }
}

/// Builds the raw adapter for `cfg` (no wrappers). API keys come from `lookup` (the process
/// environment or a secret manager), never from repository configuration.
pub fn build_raw(
    cfg: &EmbeddingConfig,
    privacy: Privacy,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Box<dyn EmbeddingProvider>, Error> {
    let provider = cfg.effective_provider(privacy);
    // A forced fallback ignores the external model's name and dims.
    let (model, dims) = if provider == cfg.provider {
        (cfg.model.as_deref(), cfg.dims)
    } else {
        (None, None)
    };
    Ok(match provider {
        ProviderName::Hash => Box::new(HashProvider::new(
            dims.unwrap_or(super::hash::DEFAULT_DIMS),
            cfg.version,
        )?),
        ProviderName::Openai => Box::new(OpenAiProvider::new(
            lookup(super::openai::API_KEY_ENV).map(Secret::new),
            model,
            dims,
            cfg.version,
            cfg.concurrency,
        )?),
        ProviderName::Voyage => Box::new(VoyageProvider::new(
            lookup(super::voyage::API_KEY_ENV).map(Secret::new),
            model,
            dims,
            cfg.version,
            cfg.concurrency,
        )?),
    })
}

/// Builds the configured provider wrapped by [`standard`]. Construction fails when a remote
/// provider has no key; the worker then runs without semantic sync.
pub fn build_provider(
    cfg: &EmbeddingConfig,
    privacy: Privacy,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Arc<dyn EmbeddingProvider>, Error> {
    Ok(standard(build_raw(cfg, privacy, lookup)?))
}
