//! Anthropic Messages API adapter (GW-003): structured output by forced tool use and prompt
//! caching with `cache_control`.

use async_trait::async_trait;
use telemetry::Secret;
use tracing::Instrument;

use super::anthropic_wire::{build_request, ContentBlock, MessagesResponse, TOOL_NAME};
use super::http::{build_client, check_base_url, execute, unexpected};
use crate::adapter::{ProviderAdapter, ProviderRequest, ProviderResponse};
use crate::error::{Error, GatewayError};
use crate::types::{FinishReason, ModelOutput, ProviderId, SchemaErrorSummary, Usage};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicAdapter {
    client: reqwest::Client,
    base_url: String,
    api_key: Secret,
}

impl std::fmt::Debug for AnthropicAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicAdapter")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .finish()
    }
}

impl AnthropicAdapter {
    pub fn new(api_key: Secret, base_url: Option<String>) -> Result<Self, Error> {
        let base_url = base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        let client = build_client().map_err(|e| Error::Config(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
        })
    }

    /// Registers only when `ANTHROPIC_API_KEY` is set. `ANTHROPIC_BASE_URL` overrides the host
    /// (rejected in production unless it is https).
    pub fn from_env() -> Result<Option<Self>, Error> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, Error> {
        let Some(key) = lookup("ANTHROPIC_API_KEY").filter(|k| !k.trim().is_empty()) else {
            return Ok(None);
        };
        let base = lookup("ANTHROPIC_BASE_URL").filter(|b| !b.trim().is_empty());
        if let Some(b) = &base {
            check_base_url(b, lookup("RG_ENV").as_deref()).map_err(Error::Config)?;
        }
        Self::new(Secret::new(key.trim()), base).map(Some)
    }
}

fn map_response(
    provider: &ProviderId,
    body: serde_json::Value,
    has_schema: bool,
    request_id: Option<String>,
) -> Result<ProviderResponse, GatewayError> {
    let parsed: MessagesResponse = serde_json::from_value(body)
        .map_err(|e| unexpected(provider, &format!("unexpected response shape: {e}")))?;
    let usage = Usage {
        input_uncached: parsed.usage.input_tokens,
        cache_write: parsed.usage.cache_creation_input_tokens,
        cache_read: parsed.usage.cache_read_input_tokens,
        output: parsed.usage.output_tokens,
        reasoning: 0,
    };
    let finish_reason = match parsed.stop_reason.as_deref() {
        Some("max_tokens") => FinishReason::MaxTokens,
        Some("refusal") => FinishReason::Refusal,
        Some("tool_use" | "end_turn" | "stop_sequence") | None => FinishReason::Complete,
        Some(other) => FinishReason::Other(other.to_owned()),
    };
    let model = parsed.model.unwrap_or_default();

    let text: String = parsed
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    if !has_schema {
        return Ok(ProviderResponse {
            output: ModelOutput::Text(text),
            usage,
            finish_reason,
            model,
            provider_request_id: request_id,
            tool_use_id: None,
        });
    }

    let tool = parsed.content.into_iter().find_map(|b| match b {
        ContentBlock::ToolUse { id, name, input } if name == TOOL_NAME => Some((id, input)),
        _ => None,
    });
    match tool {
        Some((id, input)) => Ok(ProviderResponse {
            output: ModelOutput::Json(input),
            usage,
            finish_reason,
            model,
            provider_request_id: request_id,
            tool_use_id: Some(id),
        }),
        None if matches!(
            finish_reason,
            FinishReason::Refusal | FinishReason::MaxTokens
        ) =>
        {
            Ok(ProviderResponse {
                output: ModelOutput::Text(text),
                usage,
                finish_reason,
                model,
                provider_request_id: request_id,
                tool_use_id: None,
            })
        }
        None => Err(GatewayError::SchemaViolation {
            errors: vec![SchemaErrorSummary {
                instance_path: String::new(),
                keyword: "tool_use".into(),
                message: "response contained no emit_result tool call".into(),
            }],
            repaired: false,
        }),
    }
}

#[async_trait]
impl ProviderAdapter for AnthropicAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::new(ProviderId::ANTHROPIC)
    }

    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        let provider = self.provider();
        let body = build_request(req.request, &req.candidate.model);
        let span = tracing::info_span!(
            "model_request.http",
            "gen_ai.system" = "anthropic",
            "gen_ai.request.model" = %req.candidate.model,
            "http.response.status_code" = tracing::field::Empty,
            "provider.request_id" = tracing::field::Empty,
            "gen_ai.usage.input_tokens" = tracing::field::Empty,
            "gen_ai.usage.output_tokens" = tracing::field::Empty,
        );
        let builder = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", API_VERSION)
            .timeout(req.timeout)
            .json(&body);
        let ok = execute(&provider, builder, "request-id", &span)
            .instrument(span.clone())
            .await?;
        let resp = map_response(
            &provider,
            ok.body,
            req.request.output_schema.is_some(),
            ok.request_id,
        )?;
        span.record(
            "gen_ai.usage.input_tokens",
            resp.usage.input_uncached + resp.usage.cache_read + resp.usage.cache_write,
        );
        span.record("gen_ai.usage.output_tokens", resp.usage.output);
        Ok(resp)
    }
}
