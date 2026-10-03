//! Recording decorator (GW-005): wraps a live adapter and writes replay fixtures.
//!
//! Safety rails: it exists only when `MODEL_GATEWAY_RECORD=1`; it serves only the fixture
//! organisation ([`FIXTURE_ORG_ID`]), so fixtures can never be recorded from customer
//! repositories; it writes only schema-valid, complete outputs free of secret patterns; and it
//! never overwrites an existing fixture unless `MODEL_GATEWAY_RECORD_OVERWRITE=1`.

use std::sync::Arc;

use async_trait::async_trait;
use review_core::ids::OrganizationId;

use crate::adapter::{ProviderAdapter, ProviderRequest, ProviderResponse};
use crate::error::{Error, GatewayError, PermanentKind};
use crate::fixture::{Fixture, FixtureResponse, FixtureStore, FIXTURE_VERSION};
use crate::types::{FinishReason, ModelOutput, ProviderId};
use crate::validate::SchemaValidators;

/// The only organisation whose calls may be recorded.
pub const FIXTURE_ORG_ID: OrganizationId = OrganizationId::from_uuid(uuid::Uuid::from_u128(
    0x0000_0000_0000_7000_8000_0000_0000_f1c7,
));

#[derive(Debug, Clone, Copy, Default)]
pub struct RecordConfig {
    /// `MODEL_GATEWAY_RECORD=1`.
    pub enabled: bool,
    /// `MODEL_GATEWAY_RECORD_OVERWRITE=1`.
    pub overwrite: bool,
}

impl RecordConfig {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let on = |k: &str| lookup(k).as_deref() == Some("1");
        Self {
            enabled: on("MODEL_GATEWAY_RECORD"),
            overwrite: on("MODEL_GATEWAY_RECORD_OVERWRITE"),
        }
    }
}

pub struct RecordingAdapter {
    inner: Arc<dyn ProviderAdapter>,
    store: Arc<FixtureStore>,
    config: RecordConfig,
    validators: SchemaValidators,
    git_sha: Option<String>,
}

impl std::fmt::Debug for RecordingAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingAdapter")
            .field("provider", &self.inner.provider())
            .field("overwrite", &self.config.overwrite)
            .finish()
    }
}

impl RecordingAdapter {
    /// Fails unless recording was explicitly enabled.
    pub fn new(
        inner: Arc<dyn ProviderAdapter>,
        store: Arc<FixtureStore>,
        config: RecordConfig,
    ) -> Result<Self, Error> {
        if !config.enabled {
            return Err(Error::Config(
                "record mode requires MODEL_GATEWAY_RECORD=1".into(),
            ));
        }
        Ok(Self {
            inner,
            store,
            config,
            validators: SchemaValidators::new(),
            git_sha: std::env::var("RG_GIT_SHA").ok().filter(|s| !s.is_empty()),
        })
    }

    fn recordable(&self, req: &ProviderRequest<'_>, resp: &ProviderResponse) -> bool {
        if resp.finish_reason != FinishReason::Complete {
            return false;
        }
        let text = match &resp.output {
            ModelOutput::Json(v) => {
                if let Some(schema) = &req.request.output_schema {
                    match self.validators.validate(schema, v) {
                        Ok(errors) if errors.is_empty() => {}
                        _ => return false,
                    }
                }
                v.to_string()
            }
            ModelOutput::Text(t) => t.clone(),
        };
        crate::redact::redact_str(&text).1.is_empty()
    }
}

#[async_trait]
impl ProviderAdapter for RecordingAdapter {
    fn provider(&self) -> ProviderId {
        self.inner.provider()
    }

    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        if req.request.tenant.organization_id != FIXTURE_ORG_ID {
            return Err(GatewayError::Permanent {
                kind: PermanentKind::InvalidRequest,
                provider: Some(self.inner.provider()),
                detail: "record mode only serves the fixture organisation".into(),
            });
        }
        let started = tokio::time::Instant::now();
        let resp = self.inner.send(req).await?;
        let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);

        if self.recordable(req, &resp) {
            let fixture = Fixture {
                fixture_version: FIXTURE_VERSION,
                request_hash: req.request_hash.0.clone(),
                provider: self.inner.provider().0,
                model: resp.model.clone(),
                task: req.request.task.as_str().to_owned(),
                prompt_id: req.request.input.system.prompt_id.clone(),
                prompt_version: req.request.input.system.prompt_version.clone(),
                schema_hash: req.request.output_schema.as_ref().map(|s| s.hash.clone()),
                synthetic: false,
                recorded_at: chrono::Utc::now().to_rfc3339(),
                engine_git_sha: self.git_sha.clone(),
                response: FixtureResponse {
                    output: resp.output.clone(),
                    usage: resp.usage,
                    finish_reason: resp.finish_reason.clone(),
                },
                latency_ms,
            };
            if let Err(e) = self
                .store
                .write(req.request.task, &fixture, self.config.overwrite)
                .await
            {
                tracing::warn!(error = %e, "failed to write replay fixture");
            }
        } else {
            tracing::warn!(
                request_hash = %req.request_hash,
                "response not recorded: invalid, incomplete or contains a secret pattern"
            );
        }
        Ok(resp)
    }
}
