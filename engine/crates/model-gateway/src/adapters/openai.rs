//! OpenAI Responses API adapter (GW-004): strict `json_schema` structured output.

use async_trait::async_trait;
use telemetry::Secret;
use tracing::Instrument;

use super::http::{build_client, check_base_url, execute, unexpected};
use super::openai_wire::{build_request, ContentPart, OutputItem, ResponsesResponse};
use crate::adapter::{ProviderAdapter, ProviderRequest, ProviderResponse};
use crate::error::{Error, GatewayError, PermanentKind};
use crate::schema_strict::check_strict_compatible;
use crate::types::{FinishReason, ModelOutput, ProviderId, SchemaErrorSummary, ServedFrom, Usage};

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com";

pub struct OpenAiAdapter {
    client: reqwest::Client,
    base_url: String,
    api_key: Secret,
}

impl std::fmt::Debug for OpenAiAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiAdapter")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .finish()
    }
}

impl OpenAiAdapter {
    pub fn new(api_key: Secret, base_url: Option<String>) -> Result<Self, Error> {
        let base_url = base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        let client = build_client().map_err(|e| Error::Config(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
        })
    }

    /// Registers only when `OPENAI_API_KEY` is set; `OPENAI_BASE_URL` overrides the host.
    pub fn from_env() -> Result<Option<Self>, Error> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, Error> {
        let Some(key) = lookup("OPENAI_API_KEY").filter(|k| !k.trim().is_empty()) else {
            return Ok(None);
        };
        let base = lookup("OPENAI_BASE_URL").filter(|b| !b.trim().is_empty());
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
    let parsed: ResponsesResponse = serde_json::from_value(body)
        .map_err(|e| unexpected(provider, &format!("unexpected response shape: {e}")))?;
    let cached = parsed.usage.input_tokens_details.cached_tokens;
    // OpenAI's `input_tokens` includes cached tokens.
    let usage = Usage {
        input_uncached: parsed.usage.input_tokens.saturating_sub(cached),
        cache_write: 0,
        cache_read: cached,
        output: parsed.usage.output_tokens,
        reasoning: parsed.usage.output_tokens_details.reasoning_tokens,
    };
    let model = parsed.model.clone().unwrap_or_default();

    let mut saw_message = false;
    let mut text = String::new();
    let mut refusal: Option<String> = None;
    for item in parsed.output {
        if let OutputItem::Message { content } = item {
            saw_message = true;
            for part in content {
                match part {
                    ContentPart::OutputText { text: t } => text.push_str(&t),
                    ContentPart::Refusal { refusal: r } => refusal = Some(r),
                    ContentPart::Other => {}
                }
            }
        }
    }

    let finish_reason = if refusal.is_some() {
        FinishReason::Refusal
    } else if parsed.status.as_deref() == Some("incomplete") {
        match parsed.incomplete_details.and_then(|d| d.reason).as_deref() {
            Some("max_output_tokens") => FinishReason::MaxTokens,
            Some("content_filter") => FinishReason::ContentFilter,
            other => FinishReason::Other(other.unwrap_or("incomplete").to_owned()),
        }
    } else {
        FinishReason::Complete
    };

    let make = |output, finish_reason| ProviderResponse {
        output,
        usage,
        finish_reason,
        model: model.clone(),
        provider_request_id: request_id.clone(),
        tool_use_id: None,
        served_from: ServedFrom::Live,
    };

    if let Some(r) = refusal {
        return Ok(make(ModelOutput::Text(r), FinishReason::Refusal));
    }
    if !saw_message {
        // A truncated or filtered response may legitimately carry no message item.
        if matches!(
            finish_reason,
            FinishReason::MaxTokens | FinishReason::ContentFilter
        ) {
            return Ok(make(ModelOutput::Text(String::new()), finish_reason));
        }
        return Err(unexpected(provider, "response contained no message item"));
    }
    if !has_schema || matches!(finish_reason, FinishReason::MaxTokens) {
        return Ok(make(ModelOutput::Text(text), finish_reason));
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => Ok(make(ModelOutput::Json(v), finish_reason)),
        Err(e) => Err(GatewayError::SchemaViolation {
            errors: vec![SchemaErrorSummary {
                instance_path: String::new(),
                keyword: "json".into(),
                message: format!("output_text is not valid JSON: {e}")
                    .chars()
                    .take(200)
                    .collect(),
            }],
            repaired: false,
        }),
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::new(ProviderId::OPENAI)
    }

    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        let provider = self.provider();
        if let Some(schema) = &req.request.output_schema {
            if let Err(errors) = check_strict_compatible(&schema.schema) {
                return Err(GatewayError::Permanent {
                    kind: PermanentKind::UnsupportedParameter,
                    provider: Some(provider),
                    detail: format!(
                        "schema `{}` is not strict-compatible: {}",
                        schema.name,
                        errors.join("; ")
                    )
                    .chars()
                    .take(500)
                    .collect(),
                });
            }
        }
        let body = build_request(req.request, req.candidate);
        let span = tracing::info_span!(
            "model_request.http",
            "gen_ai.system" = "openai",
            "gen_ai.request.model" = %req.candidate.model,
            "http.response.status_code" = tracing::field::Empty,
            "provider.request_id" = tracing::field::Empty,
            "openai.response_id" = tracing::field::Empty,
            "gen_ai.usage.input_tokens" = tracing::field::Empty,
            "gen_ai.usage.output_tokens" = tracing::field::Empty,
        );
        let builder = self
            .client
            .post(format!("{}/v1/responses", self.base_url))
            .bearer_auth(self.api_key.expose())
            .timeout(req.timeout)
            .json(&body);
        let ok = execute(&provider, builder, "x-request-id", &span)
            .instrument(span.clone())
            .await?;
        if let Some(id) = ok.body.get("id").and_then(serde_json::Value::as_str) {
            span.record("openai.response_id", id);
        }
        let resp = map_response(
            &provider,
            ok.body,
            req.request.output_schema.is_some(),
            ok.request_id,
        )?;
        span.record(
            "gen_ai.usage.input_tokens",
            resp.usage.input_uncached + resp.usage.cache_read,
        );
        span.record("gen_ai.usage.output_tokens", resp.usage.output);
        Ok(resp)
    }
}
