//! Shared model-call machinery of the specialist reviewers (security, test, architecture,
//! performance, maintainability). Each reviewer supplies its prompt, schema, task type, routing
//! predicate and extra input sections; the call, parsing and validation flow is the same as the
//! correctness reviewer's.

use std::sync::Arc;

use async_trait::async_trait;
use model_gateway::{
    CachePolicy, InputSection, ModelGateway, ModelOutput, ModelRequest, ModelTier, OutputSchema,
    StructuredInput, SystemPrompt, TaskType,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::error::ReviewerError;
use crate::gates::apply_gates;
use crate::input::build_input;
use crate::normalize::RefValidator;
use crate::output::{parse_output, ModelResponseMeta, ReviewerOutput};
use crate::prompts::{Prompt, PromptRef};
use crate::refs::RefTable;
use crate::reviewer::{
    Applicability, ContextBudget, ReviewRequest, Reviewer, ReviewerKind, RiskAssessment,
    RoutingInput,
};
use crate::routing::tier_for;

/// Routing predicate of one specialist reviewer.
pub type AppliesFn = fn(&RoutingInput<'_>) -> Applicability;

/// Extra (reviewer specific) input sections.
pub type ExtraSectionsFn = fn(&ReviewRequest<'_>) -> Vec<InputSection>;

/// Precondition checked before any model call. A reviewer that must not call the model without
/// a deterministic seed (maintainability) fails here, so no tokens are spent.
pub type ReadyFn = fn(&ReviewRequest<'_>) -> Result<(), ReviewerError>;

/// A reviewer defined by data and a few small functions.
#[derive(Debug, Clone)]
pub struct SpecialistReviewer {
    kind: ReviewerKind,
    task: TaskType,
    version: &'static str,
    prompt: Prompt,
    schema: OutputSchema,
    applies: AppliesFn,
    extra: ExtraSectionsFn,
    ready: ReadyFn,
    budget: fn(&RiskAssessment) -> ContextBudget,
}

/// The static description of a specialist reviewer.
#[derive(Debug, Clone)]
pub struct SpecialistSpec {
    pub kind: ReviewerKind,
    pub task: TaskType,
    pub version: &'static str,
    pub applies: AppliesFn,
    pub extra: ExtraSectionsFn,
    pub ready: ReadyFn,
    pub budget: fn(&RiskAssessment) -> ContextBudget,
}

impl SpecialistReviewer {
    pub fn new(spec: SpecialistSpec, prompt: Prompt, schema: OutputSchema) -> Self {
        Self {
            kind: spec.kind,
            task: spec.task,
            version: spec.version,
            prompt,
            schema,
            applies: spec.applies,
            extra: spec.extra,
            ready: spec.ready,
            budget: spec.budget,
        }
    }

    /// Builds the exact model request this reviewer sends.
    pub fn build_request(
        &self,
        req: &ReviewRequest<'_>,
        tier: ModelTier,
    ) -> Result<(ModelRequest, Arc<RefTable>), ReviewerError> {
        let input = build_input(self.task, req.focus, req.context, (self.extra)(req));
        let system = SystemPrompt {
            prompt_id: self.prompt.reference.kind.clone(),
            prompt_version: self.prompt.reference.prompt_version(),
            prompt_sha: self.prompt.reference.sha.clone(),
            text: Arc::from(self.prompt.render(req.focus)),
        };
        let model_req = ModelRequest::new(
            self.task,
            tier,
            StructuredInput::new(system, input.sections),
            req.tenant,
            req.budget,
        )
        .with_schema(self.schema.clone())
        .with_validator(Arc::new(RefValidator::new(Arc::clone(&input.refs))))
        .with_cache(CachePolicy::PromptOnly)
        .with_privacy(req.privacy)
        .with_risk_band(req.risk.level)
        .with_trace(req.trace.clone());
        Ok((model_req, input.refs))
    }
}

#[async_trait]
impl Reviewer for SpecialistReviewer {
    fn kind(&self) -> ReviewerKind {
        self.kind
    }

    fn version(&self) -> &'static str {
        self.version
    }

    fn prompt(&self) -> &PromptRef {
        &self.prompt.reference
    }

    fn applies(&self, input: &RoutingInput<'_>) -> Applicability {
        (self.applies)(input)
    }

    fn budget(&self, risk: &RiskAssessment) -> ContextBudget {
        (self.budget)(risk)
    }

    fn tier(&self, risk: &RiskAssessment) -> ModelTier {
        tier_for(self.kind, Some(risk))
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
        (self.ready)(&req)?;
        let span = tracing::info_span!(
            "reviewer_execution",
            reviewer_type = self.kind.as_str(),
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
            ModelOutput::Text(_) => (
                Vec::new(),
                Some("model returned no structured output".into()),
            ),
        };
        // Reviewer-specific gates (evidence, basis, path, seed): a candidate that fails one is
        // dropped and recorded, never published (silence over speculation).
        let (raw, gated) = apply_gates(self.kind, raw, req.context, &refs);
        let no_findings_reason = match no_findings_reason {
            None if raw.is_empty() && !gated.is_empty() => Some(format!(
                "{} candidate(s) dropped by the {} reviewer gates",
                gated.len(),
                self.kind.as_str()
            )),
            other => other,
        };
        Ok(ReviewerOutput {
            raw,
            no_findings_reason,
            model_response_meta: ModelResponseMeta::of(&resp),
            ref_table: refs,
            gated,
        })
    }
}

/// For reviewers that add no sections beyond the common ones.
pub fn no_extra_sections(_req: &ReviewRequest<'_>) -> Vec<InputSection> {
    Vec::new()
}

/// For reviewers that need no precondition beyond a non-empty context.
pub fn always_ready(_req: &ReviewRequest<'_>) -> Result<(), ReviewerError> {
    Ok(())
}

/// True when the cluster carries any of the named signals.
pub(crate) fn any_signal(input: &RoutingInput<'_>, set: &[&str]) -> bool {
    set.iter().any(|s| input.cluster.signals.contains(*s))
}
