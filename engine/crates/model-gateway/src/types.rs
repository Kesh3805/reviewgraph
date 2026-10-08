//! Provider-neutral request/response contract (GW-001).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use review_core::ids::{OrganizationId, RepositoryId, ReviewRunId, ReviewerRunId};
use review_core::reviewer_type::ReviewerType;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

/// Capability tier a caller asks for. Callers never name a model (ADR-010).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Classifier,
    FastReasoner,
    ReviewReasoner,
    Verifier,
    DeepReasoner,
}

impl ModelTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classifier => "classifier",
            Self::FastReasoner => "fast_reasoner",
            Self::ReviewReasoner => "review_reasoner",
            Self::Verifier => "verifier",
            Self::DeepReasoner => "deep_reasoner",
        }
    }
}

/// What the call is for. Drives response-cache eligibility and metric labels.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    IntentClassification,
    SymbolSummary,
    CorrectnessReview,
    SecurityReview,
    TestReview,
    ArchitectureReview,
    PerformanceReview,
    MaintainabilityReview,
    ContradictionAdjudication,
    EvalProbe,
}

impl TaskType {
    /// Only deterministic, input-pure tasks may be served from the response cache.
    pub const fn response_cacheable(self) -> bool {
        matches!(
            self,
            Self::IntentClassification | Self::SymbolSummary | Self::ContradictionAdjudication
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IntentClassification => "intent_classification",
            Self::SymbolSummary => "symbol_summary",
            Self::CorrectnessReview => "correctness_review",
            Self::SecurityReview => "security_review",
            Self::TestReview => "test_review",
            Self::ArchitectureReview => "architecture_review",
            Self::PerformanceReview => "performance_review",
            Self::MaintainabilityReview => "maintainability_review",
            Self::ContradictionAdjudication => "contradiction_adjudication",
            Self::EvalProbe => "eval_probe",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    Off,
    Low,
    Medium,
    High,
}

/// Risk band of the change under review (RISK-001 will own the canonical type).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskBand {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CachePolicy {
    /// No provider prompt-cache markers and no response cache.
    Disabled,
    /// Provider prompt caching only.
    PromptOnly,
    /// Prompt caching plus the tenant-scoped response cache for cacheable tasks.
    PromptAndResponse { ttl: Duration },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    Standard,
    ZeroRetentionOnly,
    NoExternal,
}

/// Tenant a call is made for. Mandatory: no `Default`, and `ModelRequest` has no constructor
/// that omits it.
///
/// ```compile_fail
/// let _ = model_gateway::TenantScope::default();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct TenantScope {
    pub organization_id: OrganizationId,
    pub repository_id: RepositoryId,
}

/// Per-call limits. The run-level ledger (PIPE-006) derives one for every call.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
pub struct CallBudget {
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_cost_usd_micros: Option<u64>,
    #[serde(skip)]
    #[schemars(skip)]
    pub deadline: Instant,
    /// Share of the run budget still unspent (1.0 = all); lets the router gate `deep_reasoner`.
    pub remaining_fraction: f32,
    pub max_attempts: u8,
}

impl CallBudget {
    /// A budget that expires `timeout` from now.
    pub fn within(timeout: Duration) -> Self {
        Self {
            max_input_tokens: 200_000,
            max_output_tokens: 16_000,
            max_cost_usd_micros: None,
            deadline: Instant::now() + timeout,
            remaining_fraction: 1.0,
            max_attempts: 3,
        }
    }
}

/// Identifies the system prompt that produced a request.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SystemPrompt {
    pub prompt_id: String,
    pub prompt_version: String,
    pub prompt_sha: String,
    #[schemars(with = "String")]
    pub text: Arc<str>,
}

/// One named, ordered part of the user input. Most stable sections go first so provider
/// prompt caches hit.
#[derive(Clone, Serialize, JsonSchema)]
pub struct InputSection {
    pub name: String,
    pub cache_breakpoint: bool,
    pub content: serde_json::Value,
}

impl InputSection {
    pub fn new(name: impl Into<String>, content: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            cache_breakpoint: false,
            content,
        }
    }

    pub fn with_cache_breakpoint(mut self) -> Self {
        self.cache_breakpoint = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SchemaErrorSummary {
    pub instance_path: String,
    pub keyword: String,
    pub message: String,
}

/// A repair turn appended by the gateway after an invalid structured output (GW-009).
///
/// Only `previous_output` and `errors` enter the request hash; `tool_use_id` is provider wire
/// detail (Anthropic needs it to answer the original `tool_use` block).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RepairTurn {
    pub previous_output: serde_json::Value,
    pub errors: Vec<SchemaErrorSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
}

/// Caller-supplied semantic checks run after JSON Schema validation (GW-009), for example the
/// reviewer ref-existence validator (REV-C-003). Errors must never echo instance values.
pub trait OutputValidator: Send + Sync + fmt::Debug {
    fn validate(&self, output: &serde_json::Value) -> Vec<SchemaErrorSummary>;
}

/// The input of a call. `Debug` prints only section names and byte lengths.
#[derive(Clone, Serialize, JsonSchema)]
pub struct StructuredInput {
    pub system: SystemPrompt,
    pub sections: Vec<InputSection>,
    pub repair: Option<RepairTurn>,
}

impl StructuredInput {
    pub fn new(system: SystemPrompt, sections: Vec<InputSection>) -> Self {
        Self {
            system,
            sections,
            repair: None,
        }
    }

    /// Approximate size in bytes of everything that would be sent.
    pub fn byte_len(&self) -> usize {
        self.system.text.len()
            + self
                .sections
                .iter()
                .map(|s| s.name.len() + s.content.to_string().len())
                .sum::<usize>()
    }
}

impl fmt::Debug for InputSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputSection")
            .field("name", &self.name)
            .field("bytes", &self.content.to_string().len())
            .field("cache_breakpoint", &self.cache_breakpoint)
            .finish()
    }
}

impl fmt::Debug for StructuredInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StructuredInput")
            .field("prompt_id", &self.system.prompt_id)
            .field("prompt_version", &self.system.prompt_version)
            .field("system_bytes", &self.system.text.len())
            .field("sections", &self.sections)
            .field("repair", &self.repair.is_some())
            .finish()
    }
}

/// JSON Schema the output must satisfy.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OutputSchema {
    pub name: String,
    pub version: String,
    #[schemars(with = "serde_json::Value")]
    pub schema: Arc<serde_json::Value>,
    pub hash: String,
}

impl OutputSchema {
    /// Computes `hash` as blake3 over the canonical (JCS) schema.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        schema: serde_json::Value,
    ) -> Self {
        let hash = crate::request_hash::hash_value(&schema);
        Self {
            name: name.into(),
            version: version.into(),
            schema: Arc::new(schema),
            hash,
        }
    }
}

/// Correlation data for telemetry. Never hashed.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct TraceContext {
    pub review_run_id: Option<ReviewRunId>,
    pub reviewer_run_id: Option<ReviewerRunId>,
    pub reviewer_type: Option<ReviewerType>,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelRequest {
    pub task: TaskType,
    pub tier: ModelTier,
    pub reasoning: ReasoningLevel,
    pub risk_band: RiskBand,
    pub input: StructuredInput,
    pub output_schema: Option<OutputSchema>,
    pub max_output_tokens: u32,
    pub cache: CachePolicy,
    pub privacy: PrivacyClass,
    pub budget: CallBudget,
    pub tenant: TenantScope,
    pub trace: TraceContext,
    pub idempotency_hint: Option<String>,
    /// Semantic output checks (GW-009). Not hashed and not serialised.
    #[serde(skip)]
    #[schemars(skip)]
    pub validator: Option<Arc<dyn OutputValidator>>,
}

impl ModelRequest {
    /// Starts a request. `tenant` is mandatory; everything else has a conservative default.
    pub fn new(
        task: TaskType,
        tier: ModelTier,
        input: StructuredInput,
        tenant: TenantScope,
        budget: CallBudget,
    ) -> Self {
        Self {
            task,
            tier,
            reasoning: ReasoningLevel::Off,
            risk_band: RiskBand::Low,
            input,
            output_schema: None,
            max_output_tokens: budget.max_output_tokens,
            cache: CachePolicy::PromptOnly,
            privacy: PrivacyClass::Standard,
            budget,
            tenant,
            trace: TraceContext::default(),
            idempotency_hint: None,
            validator: None,
        }
    }

    pub fn with_validator(mut self, validator: Arc<dyn OutputValidator>) -> Self {
        self.validator = Some(validator);
        self
    }

    pub fn with_schema(mut self, schema: OutputSchema) -> Self {
        self.output_schema = Some(schema);
        self
    }

    pub fn with_reasoning(mut self, reasoning: ReasoningLevel) -> Self {
        self.reasoning = reasoning;
        self
    }

    pub fn with_risk_band(mut self, band: RiskBand) -> Self {
        self.risk_band = band;
        self
    }

    pub fn with_cache(mut self, cache: CachePolicy) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_privacy(mut self, privacy: PrivacyClass) -> Self {
        self.privacy = privacy;
        self
    }

    pub fn with_trace(mut self, trace: TraceContext) -> Self {
        self.trace = trace;
        self
    }
}

/// Token usage with provider semantics normalised: `input_uncached` excludes cache reads and
/// writes, and `output` includes reasoning tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    pub input_uncached: u32,
    pub cache_write: u32,
    pub cache_read: u32,
    pub output: u32,
    pub reasoning: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelOutput {
    Json(serde_json::Value),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Complete,
    MaxTokens,
    Refusal,
    ContentFilter,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServedFrom {
    Live,
    ResponseCache,
    Replay,
}

impl ServedFrom {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::ResponseCache => "response_cache",
            Self::Replay => "replay",
        }
    }
}

/// Provider name as configured in `routing.yaml` (`anthropic`, `openai`, `replay`, ...).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ProviderId(pub String);

impl ProviderId {
    pub const ANTHROPIC: &'static str = "anthropic";
    pub const OPENAI: &'static str = "openai";
    pub const REPLAY: &'static str = "replay";

    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// blake3 of the canonical request (see `request_hash`), lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RequestHash(pub String);

impl RequestHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One routable (provider, model) pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteCandidate {
    pub provider: ProviderId,
    pub model: String,
    pub max_context: u32,
    #[serde(default)]
    pub supports_reasoning: bool,
}

/// A fallback that was attempted and why it did not serve the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttemptedCandidate {
    pub provider: ProviderId,
    pub model: String,
    pub error_class: String,
}

/// How a call was routed. Persisted by consumers (`reviewer_runs.route`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteDecision {
    pub requested_tier: ModelTier,
    pub effective_tier: ModelTier,
    pub candidates: Vec<RouteCandidate>,
    pub downgraded: Option<String>,
    pub table_hash: String,
    #[serde(default)]
    pub attempted: Vec<AttemptedCandidate>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelResponse {
    pub output: ModelOutput,
    pub usage: Usage,
    pub latency_ms: u32,
    pub provider: ProviderId,
    pub model: String,
    pub cost_usd_micros: Option<u64>,
    pub finish_reason: FinishReason,
    pub request_hash: RequestHash,
    pub route: RouteDecision,
    pub attempts: u8,
    pub served_from: ServedFrom,
    /// On a response-cache hit: the usage of the call that produced the cached output (`usage`
    /// is zero because nothing was billed).
    pub usage_original: Option<Usage>,
}
