//! Embedding provider port (SEM-001) and adapters (SEM-002).
//!
//! Callers never use a raw adapter: [`standard`] wraps it as
//! `Redacting<RetryingProvider<Normalized<P>>>`, so secrets are scrubbed before any provider call,
//! transient failures are retried once per policy, and every vector is checked and L2-normalized.

pub mod config;
pub mod error;
pub mod hash;
mod http;
pub mod openai;
pub mod space;
pub mod voyage;
mod wrap;

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;

pub use config::{build_provider, build_raw, EmbeddingConfig, Privacy};
pub use error::EmbedError;
pub use hash::HashProvider;
pub use openai::OpenAiProvider;
pub use space::{EmbeddingSpace, ProviderName};
pub use voyage::VoyageProvider;
pub use wrap::{l2_normalize, Jitter, Normalized, Redacting, RetryPolicy, RetryingProvider};

/// Correlation attributes carried into the `embedding_request` span.
pub type TraceContext = telemetry::attrs::Correlation;

/// Asymmetric models embed documents and queries differently (Voyage `input_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputKind {
    Document,
    Query,
}

impl InputKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Query => "query",
        }
    }
}

/// One batch of texts to embed.
#[derive(Debug, Clone)]
pub struct EmbedRequest<'a> {
    pub kind: InputKind,
    pub texts: &'a [String],
    pub trace: TraceContext,
}

impl<'a> EmbedRequest<'a> {
    pub fn new(kind: InputKind, texts: &'a [String]) -> Self {
        Self {
            kind,
            texts,
            trace: TraceContext::default(),
        }
    }
}

/// Vectors in input order. After [`Normalized`] every vector has unit length.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbedResponse {
    pub vectors: Vec<Vec<f32>>,
    pub usage_tokens: u32,
    pub latency_ms: u32,
}

/// An embedding model. Shared as `Arc<dyn EmbeddingProvider>`.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync + Debug {
    fn space(&self) -> &EmbeddingSpace;
    /// Maximum number of texts per request.
    fn max_batch(&self) -> usize;
    /// Maximum estimated tokens per text (`ceil(chars / 4)`).
    fn max_input_tokens(&self) -> usize;
    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError>;
}

#[async_trait]
impl<P: EmbeddingProvider + ?Sized> EmbeddingProvider for Arc<P> {
    fn space(&self) -> &EmbeddingSpace {
        (**self).space()
    }

    fn max_batch(&self) -> usize {
        (**self).max_batch()
    }

    fn max_input_tokens(&self) -> usize {
        (**self).max_input_tokens()
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        (**self).embed(req).await
    }
}

#[async_trait]
impl<P: EmbeddingProvider + ?Sized> EmbeddingProvider for Box<P> {
    fn space(&self) -> &EmbeddingSpace {
        (**self).space()
    }

    fn max_batch(&self) -> usize {
        (**self).max_batch()
    }

    fn max_input_tokens(&self) -> usize {
        (**self).max_input_tokens()
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        (**self).embed(req).await
    }
}

/// Token estimate used for batching and budgets: `ceil(chars / 4)`.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Returns `InputTooLong` for the first text over `max_tokens`.
pub fn check_input_lengths(texts: &[String], max_tokens: usize) -> Result<(), EmbedError> {
    match texts.iter().position(|t| estimate_tokens(t) > max_tokens) {
        Some(index) => Err(EmbedError::InputTooLong { index }),
        None => Ok(()),
    }
}

/// The production stack around a raw adapter: redaction, retry, normalization and metrics.
pub fn standard<P: EmbeddingProvider + 'static>(provider: P) -> Arc<dyn EmbeddingProvider> {
    Arc::new(Redacting::new(RetryingProvider::new(
        Normalized::new(provider),
        RetryPolicy::default(),
    )))
}
