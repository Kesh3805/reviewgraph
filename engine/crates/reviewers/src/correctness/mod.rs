//! The correctness reviewer: one model call per cluster (REV-C-001 binding, REV-C-002 core).
//!
//! Implemented here: request construction (prompt v1, `reviewer_output.v1`, the ref validator),
//! the gateway call and parsing. The cluster fan-out, `reviewer_runs` persistence and the
//! stage-output cache of REV-C-002 wait for PIPE-005/006.

pub mod prompt;

use std::sync::Arc;

use async_trait::async_trait;
use model_gateway::{
    CachePolicy, ModelGateway, ModelOutput, ModelRequest, ModelTier, StructuredInput, TaskType,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::error::ReviewerError;
use crate::focus::active_profiles;
use crate::input::build_input;
use crate::normalize::RefValidator;
use crate::output::{parse_output, reviewer_output_schema, ModelResponseMeta, ReviewerOutput};
use crate::prompts::{Prompt, PromptRef};
use crate::reviewer::{
    Applicability, ContextBudget, ReviewRequest, Reviewer, ReviewerKind, RiskAssessment,
    RoutingInput,
};
use crate::routing::{tier_for, SkipReason, NON_BEHAVIOURAL};

/// Semver of the correctness reviewer code.
pub const CORRECTNESS_REVIEWER_VERSION: &str = "1.0.0";

#[derive(Debug, Clone)]
pub struct CorrectnessReviewer {
    prompt: Prompt,
}

impl CorrectnessReviewer {
    /// Loads prompt v1; fails if the embedded asset is broken.
    pub fn new() -> Result<Self, ReviewerError> {
        Ok(Self {
            prompt: prompt::correctness_prompt()?,
        })
    }

    /// Builds the exact model request this reviewer sends (also used by the fixture authoring
    /// helper, so fixtures key on the same request hash).
    pub fn build_request(
        &self,
        req: &ReviewRequest<'_>,
        tier: ModelTier,
    ) -> Result<(ModelRequest, Arc<crate::refs::RefTable>), ReviewerError> {
        let input = build_input(
            TaskType::CorrectnessReview,
            req.focus,
            req.context,
            Vec::new(),
        );
        let system = prompt::system_prompt(&self.prompt, req.focus);
        let schema = reviewer_output_schema()?;
        let model_req = ModelRequest::new(
            TaskType::CorrectnessReview,
            tier,
            StructuredInput::new(system, input.sections),
            req.tenant,
            req.budget,
        )
        .with_schema(schema)
        .with_validator(Arc::new(RefValidator::new(Arc::clone(&input.refs))))
        .with_cache(CachePolicy::PromptOnly)
        .with_privacy(req.privacy)
        .with_risk_band(req.risk.level)
        .with_trace(req.trace.clone());
        Ok((model_req, input.refs))
    }
}

#[async_trait]
impl Reviewer for CorrectnessReviewer {
    fn kind(&self) -> ReviewerKind {
        ReviewerKind::Correctness
    }

    fn version(&self) -> &'static str {
        CORRECTNESS_REVIEWER_VERSION
    }

    fn prompt(&self) -> &PromptRef {
        &self.prompt.reference
    }

    fn applies(&self, input: &RoutingInput<'_>) -> Applicability {
        let signals = &input.cluster.signals;
        let behavioural = signals.is_empty()
            || signals
                .iter()
                .any(|s| !NON_BEHAVIOURAL.contains(&s.as_str()));
        if behavioural {
            Applicability::Applies {
                focus_profiles: active_profiles(signals.iter().map(String::as_str)),
            }
        } else {
            Applicability::Skip {
                reason: SkipReason::NotApplicable,
            }
        }
    }

    fn budget(&self, risk: &RiskAssessment) -> ContextBudget {
        ContextBudget::for_risk(risk.level)
    }

    fn tier(&self, risk: &RiskAssessment) -> ModelTier {
        tier_for(ReviewerKind::Correctness, Some(risk))
    }

    async fn review(
        &self,
        req: ReviewRequest<'_>,
        gw: &dyn ModelGateway,
        cancel: CancellationToken,
    ) -> Result<ReviewerOutput, ReviewerError> {
        if req.context.changed_symbols().is_empty() && req.context.nodes().is_empty() {
            return Err(ReviewerError::InvalidContext(
                "the context package has no items".into(),
            ));
        }
        let span = tracing::info_span!(
            "reviewer_execution",
            reviewer_type = "correctness",
            cluster_id = req.cluster_id,
            prompt_version = %self.prompt.reference.prompt_version(),
        );
        let (model_req, refs) = span.in_scope(|| self.build_request(&req, self.tier(req.risk)))?;
        let resp = gw
            .call(model_req, cancel)
            .instrument(span)
            .await
            .map_err(|e| match e {
                model_gateway::GatewayError::Cancelled => ReviewerError::Cancelled,
                other => ReviewerError::Gateway(other),
            })?;
        let (raw, no_findings_reason) = match &resp.output {
            ModelOutput::Json(v) => {
                let (items, reason, _dropped) = parse_output(v);
                (items, reason)
            }
            // A refusal carries text and no findings.
            ModelOutput::Text(_) => (
                Vec::new(),
                Some("model returned no structured output".into()),
            ),
        };
        Ok(ReviewerOutput {
            raw,
            no_findings_reason,
            model_response_meta: ModelResponseMeta::of(&resp),
            ref_table: refs,
        })
    }
}
