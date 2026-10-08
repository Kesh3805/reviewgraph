//! OpenAI embeddings adapter (SEM-002): `POST /v1/embeddings`.

use async_trait::async_trait;
use serde_json::json;
use telemetry::Secret;
use tokio::sync::Semaphore;

use super::http::{build_client, embed_batched, post_json, usage_tokens, vectors_by_index};
use super::{
    check_input_lengths, EmbedError, EmbedRequest, EmbedResponse, EmbeddingProvider,
    EmbeddingSpace, ProviderName,
};
use crate::error::Error;

pub const DEFAULT_MODEL: &str = "text-embedding-3-small";
pub const DEFAULT_DIMS: u16 = 1536;
pub const MAX_BATCH: usize = 256;
pub const MAX_INPUT_TOKENS: usize = 8191;
pub const API_KEY_ENV: &str = "OPENAI_API_KEY";
const BASE_URL: &str = "https://api.openai.com";

/// OpenAI adapter. `InputKind` is ignored (symmetric model).
pub struct OpenAiProvider {
    space: EmbeddingSpace,
    api_key: Secret,
    http: reqwest::Client,
    base_url: String,
    permits: Semaphore,
    /// Whether to send `dimensions` (only when not the model's native size).
    send_dimensions: bool,
}

impl std::fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("space", &self.space)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl OpenAiProvider {
    /// `model`/`dims` default to `text-embedding-3-small`/1536. Fails without an API key.
    pub fn new(
        api_key: Option<Secret>,
        model: Option<&str>,
        dims: Option<u16>,
        version: u16,
        concurrency: usize,
    ) -> Result<Self, Error> {
        let api_key = api_key
            .filter(|k| !k.expose().trim().is_empty())
            .ok_or_else(|| Error::Config(format!("{API_KEY_ENV} is not set")))?;
        let model = model.unwrap_or(DEFAULT_MODEL);
        let dims = dims.unwrap_or(DEFAULT_DIMS);
        let space = EmbeddingSpace::new(ProviderName::Openai, model, dims, version)?;
        let http = build_client().map_err(|e| Error::Config(e.to_string()))?;
        Ok(Self {
            send_dimensions: !(model == DEFAULT_MODEL && dims == DEFAULT_DIMS),
            space,
            api_key,
            http,
            base_url: BASE_URL.to_owned(),
            permits: Semaphore::new(concurrency.max(1)),
        })
    }

    /// Points the adapter at a mock server. Test builds only.
    #[cfg(any(test, feature = "test-endpoints"))]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_owned();
        self
    }

    async fn call(&self, texts: &[String]) -> Result<(Vec<Vec<f32>>, u32), EmbedError> {
        let mut body = json!({
            "model": self.space.model,
            "input": texts,
            "encoding_format": "float",
        });
        if self.send_dimensions {
            body["dimensions"] = json!(self.space.dims);
        }
        let url = format!("{}/v1/embeddings", self.base_url);
        let resp = post_json(&self.http, &url, self.api_key.expose(), &body).await?;
        Ok((vectors_by_index(&resp, texts.len())?, usage_tokens(&resp)))
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiProvider {
    fn space(&self) -> &EmbeddingSpace {
        &self.space
    }

    fn max_batch(&self) -> usize {
        MAX_BATCH
    }

    fn max_input_tokens(&self) -> usize {
        MAX_INPUT_TOKENS
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        check_input_lengths(req.texts, MAX_INPUT_TOKENS)?;
        embed_batched(req.texts, MAX_BATCH, &self.permits, "openai", |batch| {
            self.call(batch)
        })
        .await
    }
}
