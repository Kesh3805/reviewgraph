//! Provider adapter contract (GW-001). Adapters stay thin: map a [`ProviderRequest`] to the
//! provider wire format and back, and report failures as [`GatewayError`].

use std::time::Duration;

use async_trait::async_trait;

use crate::error::GatewayError;
use crate::types::{FinishReason, ModelOutput, ModelRequest, ProviderId, RouteCandidate, Usage};

/// What an adapter is asked to send.
#[derive(Debug)]
pub struct ProviderRequest<'a> {
    pub request: &'a ModelRequest,
    pub candidate: &'a RouteCandidate,
    /// Upper bound for this single HTTP attempt.
    pub timeout: Duration,
}

/// What an adapter returns on success.
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub output: ModelOutput,
    pub usage: Usage,
    pub finish_reason: FinishReason,
    /// Model id the provider reports having served.
    pub model: String,
    /// Provider request id, for support tickets (never a secret).
    pub provider_request_id: Option<String>,
    /// Id of the forced tool call (Anthropic), needed to render a repair turn.
    pub tool_use_id: Option<String>,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> ProviderId;
    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError>;
}
