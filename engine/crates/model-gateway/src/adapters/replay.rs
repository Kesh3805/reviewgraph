//! Deterministic replay adapter (GW-005): serves recorded or synthetic fixtures keyed by the
//! request hash. A miss is `Permanent(ReplayMiss)`, never fallback-eligible, so tests fail loudly
//! instead of silently reaching a live model.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::adapter::{ProviderAdapter, ProviderRequest, ProviderResponse};
use crate::error::{GatewayError, PermanentKind};
use crate::fixture::FixtureStore;
use crate::types::{ProviderId, ServedFrom};

/// How the gateway obtains model output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayMode {
    /// Network adapters (default for deployed workers).
    Live,
    /// Fixtures only (default in tests and CI).
    Replay,
    /// Live calls whose results are written as fixtures; see `RecordingAdapter`.
    Record,
}

impl GatewayMode {
    /// Parses `MODEL_GATEWAY_MODE`; `None` for an unknown value.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "live" => Some(Self::Live),
            "replay" => Some(Self::Replay),
            "record" => Some(Self::Record),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplayConfig {
    pub root: PathBuf,
    /// `REPLAY_LATENCY=recorded`: sleep the recorded latency (for perf runs).
    pub recorded_latency: bool,
    /// Where misses are appended (JSON lines).
    pub miss_log: Option<PathBuf>,
}

impl ReplayConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            recorded_latency: false,
            miss_log: Some(PathBuf::from("target/replay-misses.jsonl")),
        }
    }

    /// `MODEL_REPLAY_DIR` (default `fixtures/model-replay`) and `REPLAY_LATENCY`.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let root = lookup("MODEL_REPLAY_DIR").unwrap_or_else(|| "fixtures/model-replay".to_owned());
        let mut c = Self::new(root);
        c.recorded_latency = lookup("REPLAY_LATENCY").as_deref() == Some("recorded");
        c
    }
}

/// Serves fixtures while impersonating `provider`, so one store can back every routed provider.
#[derive(Debug)]
pub struct ReplayAdapter {
    provider: ProviderId,
    store: Arc<FixtureStore>,
    config: ReplayConfig,
}

impl ReplayAdapter {
    pub fn new(provider: ProviderId, store: Arc<FixtureStore>, config: ReplayConfig) -> Self {
        Self {
            provider,
            store,
            config,
        }
    }

    async fn log_miss(&self, req: &ProviderRequest<'_>) {
        let Some(path) = &self.config.miss_log else {
            return;
        };
        let line = serde_json::json!({
            "request_hash": req.request_hash.as_str(),
            "task": req.request.task.as_str(),
            "prompt_version": req.request.input.system.prompt_version,
            "section_names": req.request.input.sections.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        });
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        let mut text = line.to_string();
        text.push('\n');
        use tokio::io::AsyncWriteExt;
        if let Ok(mut f) = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
        {
            let _ = f.write_all(text.as_bytes()).await;
            let _ = f.flush().await;
        }
    }
}

#[async_trait]
impl ProviderAdapter for ReplayAdapter {
    fn provider(&self) -> ProviderId {
        self.provider.clone()
    }

    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        let found = self
            .store
            .lookup(
                req.request.task,
                req.request_hash.as_str(),
                self.provider.as_str(),
                &req.candidate.model,
            )
            .await?;
        let Some(fixture) = found else {
            self.log_miss(req).await;
            return Err(GatewayError::Permanent {
                kind: PermanentKind::ReplayMiss,
                provider: Some(self.provider.clone()),
                detail: format!(
                    "no replay fixture for {} (task {})",
                    req.request_hash,
                    req.request.task.as_str()
                ),
            });
        };
        if self.config.recorded_latency && fixture.latency_ms > 0 {
            tokio::time::sleep(Duration::from_millis(u64::from(fixture.latency_ms))).await;
        }
        Ok(ProviderResponse {
            output: fixture.response.output.clone(),
            usage: fixture.response.usage,
            finish_reason: fixture.response.finish_reason.clone(),
            model: if fixture.model == crate::fixture::ANY {
                req.candidate.model.clone()
            } else {
                fixture.model.clone()
            },
            provider_request_id: None,
            tool_use_id: None,
            served_from: ServedFrom::Replay,
        })
    }
}
