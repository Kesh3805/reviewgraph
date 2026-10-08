//! Security reviewer assets (REV-S-001): prompt v1, the `security.v1` output schema and the
//! security candidate extension. Context selection (REV-S-002) and deterministic checks
//! (REV-S-003) are separate tasks.

pub mod categories;

use std::sync::LazyLock;

use model_gateway::{InputSection, OutputSchema, TaskType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use categories::{MissingControl, SecurityCategory, TrustSource};

use crate::context::ReviewContext;
use crate::error::ReviewerError;
use crate::input::{build_input, ModelReviewInput};
use crate::prompts::{prompt, Prompt};

pub const SECURITY_SCHEMA_NAME: &str = "security";
pub const SECURITY_SCHEMA_VERSION: &str = "v1";
pub const SECURITY_SCHEMA_JSON: &str = include_str!("../../prompts/security/v1.schema.json");

static SECURITY_SCHEMA: LazyLock<Result<OutputSchema, String>> = LazyLock::new(|| {
    serde_json::from_str::<Value>(SECURITY_SCHEMA_JSON)
        .map(|v| OutputSchema::new(SECURITY_SCHEMA_NAME, SECURITY_SCHEMA_VERSION, v))
        .map_err(|e| e.to_string())
});

/// The `security.v1` output schema.
pub fn security_output_schema() -> Result<OutputSchema, ReviewerError> {
    SECURITY_SCHEMA
        .clone()
        .map_err(|e| ReviewerError::Prompt(format!("security.v1 schema is not valid JSON: {e}")))
}

/// The security prompt, v1.
pub fn security_prompt() -> Result<Prompt, ReviewerError> {
    prompt("security", 1)
}

/// One endpoint path a security claim relies on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryPoint {
    pub endpoint_node_id: String,
    pub path_symbol_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustBoundary {
    pub source: TrustSource,
    pub sink_symbol_key: String,
}

/// The fields `security.v1` adds to the common candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityCandidateExtension {
    pub category: SecurityCategory,
    pub entry_points: Vec<EntryPoint>,
    pub trust_boundary: Option<TrustBoundary>,
    pub missing_control: Option<MissingControl>,
}

impl SecurityCandidateExtension {
    /// Reads the extension from one raw `security.v1` finding.
    pub fn from_raw(finding: &Value) -> Option<Self> {
        serde_json::from_value(json!({
            "category": finding.get("category")?,
            "entry_points": finding.get("entry_points")?,
            "trust_boundary": finding.get("trust_boundary")?,
            "missing_control": finding.get("missing_control")?,
        }))
        .ok()
    }
}

/// A known security pattern of the repository (from PROF-004 conventions when present).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityPattern {
    pub pattern: String,
    pub description: String,
}

/// The security input: the common sections plus `security_patterns` (untrusted data).
pub fn security_input(cx: &dyn ReviewContext, patterns: &[SecurityPattern]) -> ModelReviewInput {
    let section = InputSection::new(
        "security_patterns",
        json!({ "untrusted_data": true, "patterns": patterns }),
    );
    build_input(TaskType::SecurityReview, &[], cx, vec![section])
}
