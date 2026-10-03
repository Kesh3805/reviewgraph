//! Provider adapter contract (GW-001). Adapters stay thin: map a [`ProviderRequest`] to the
//! provider wire format and back, and report failures as [`GatewayError`].

use std::time::Duration;

use async_trait::async_trait;

use crate::error::GatewayError;
use crate::types::{
    FinishReason, ModelOutput, ModelRequest, ProviderId, RequestHash, RouteCandidate, ServedFrom,
    Usage,
};

/// What an adapter is asked to send.
#[derive(Debug)]
pub struct ProviderRequest<'a> {
    pub request: &'a ModelRequest,
    pub candidate: &'a RouteCandidate,
    /// Canonical hash of `request` (after redaction).
    pub request_hash: &'a RequestHash,
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
    /// `Live` for network adapters, `Replay` for the replay adapter.
    pub served_from: ServedFrom,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> ProviderId;
    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError>;
}
