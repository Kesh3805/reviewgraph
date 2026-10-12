//! Reviewer output (REV-001): the `reviewer_output.v1` JSON Schema and the raw candidate shape the
//! model returns. Raw candidates cite refs; [`crate::normalize`] resolves them.

use std::sync::{Arc, LazyLock};

use model_gateway::{ModelResponse, OutputSchema, ServedFrom, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ReviewerError;
use crate::refs::RefTable;

/// Schema name and version used in requests and fixtures.
pub const REVIEWER_OUTPUT_SCHEMA_NAME: &str = "reviewer_output";
pub const REVIEWER_OUTPUT_SCHEMA_VERSION: &str = "v1";
/// The schema text, embedded at build time.
pub const REVIEWER_OUTPUT_SCHEMA_JSON: &str =
    include_str!("../schemas/reviewer_output.v1.schema.json");

static REVIEWER_OUTPUT: LazyLock<Result<OutputSchema, String>> = LazyLock::new(|| {
    serde_json::from_str::<Value>(REVIEWER_OUTPUT_SCHEMA_JSON)
        .map(|v| {
            OutputSchema::new(
                REVIEWER_OUTPUT_SCHEMA_NAME,
                REVIEWER_OUTPUT_SCHEMA_VERSION,
                v,
            )
        })
        .map_err(|e| e.to_string())
});

/// The common reviewer output schema (`reviewer_output.v1`).
pub fn reviewer_output_schema() -> Result<OutputSchema, ReviewerError> {
    REVIEWER_OUTPUT
        .clone()
        .map_err(|e| ReviewerError::Prompt(format!("reviewer_output.v1 is not valid JSON: {e}")))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawAnchor {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub side: String,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawEvidence {
    pub kind: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub side: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub quote: String,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRelation {
    pub from: String,
    pub relation: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawParam {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPredicate {
    pub kind: String,
    pub subject: String,
    pub params: Vec<RawParam>,
}

/// One candidate exactly as the model returned it (after schema validation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCandidate {
    pub category: String,
    pub title: String,
    pub claim: String,
    pub description: String,
    pub severity: String,
    pub anchor: RawAnchor,
    pub affected_refs: Vec<String>,
    pub evidence: Vec<RawEvidence>,
    pub claimed_relations: Vec<RawRelation>,
    pub predicate: RawPredicate,
    pub corrective_direction: String,
    /// Stored for calibration research only; never a publication input (ADR-011).
    pub self_confidence: f64,
}

/// A raw candidate with its position in the model output and its original JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct RawItem {
    pub ordinal: usize,
    pub candidate: RawCandidate,
    pub json: Value,
}

/// Facts about the model call, for `reviewer_runs` and evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelResponseMeta {
    pub provider: String,
    pub model: String,
    pub request_hash: String,
    pub usage: Usage,
    pub cost_usd_micros: Option<u64>,
    pub latency_ms: u32,
    pub attempts: u8,
    pub served_from: ServedFrom,
    pub route_table_hash: String,
}

impl ModelResponseMeta {
    pub fn of(resp: &ModelResponse) -> Self {
        Self {
            provider: resp.provider.0.clone(),
            model: resp.model.clone(),
            request_hash: resp.request_hash.0.clone(),
            usage: resp.usage,
            cost_usd_micros: resp.cost_usd_micros,
            latency_ms: resp.latency_ms,
            attempts: resp.attempts,
            served_from: resp.served_from,
            route_table_hash: resp.route.table_hash.clone(),
        }
    }
}

/// What a reviewer returns for one cluster.
#[derive(Debug, Clone)]
pub struct ReviewerOutput {
    pub raw: Vec<RawItem>,
    pub no_findings_reason: Option<String>,
    pub model_response_meta: ModelResponseMeta,
    pub ref_table: Arc<RefTable>,
}

/// Splits a validated model output into raw items. An item that does not deserialise (possible
/// only for semantic reasons the schema cannot express) is skipped and counted by the caller via
/// `dropped`.
pub fn parse_output(output: &Value) -> (Vec<RawItem>, Option<String>, usize) {
    let reason = output
        .get("no_findings_reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut items = Vec::new();
    let mut dropped = 0;
    if let Some(findings) = output.get("findings").and_then(Value::as_array) {
        for (ordinal, json) in findings.iter().enumerate() {
            match serde_json::from_value::<RawCandidate>(json.clone()) {
                Ok(candidate) => items.push(RawItem {
                    ordinal,
                    candidate,
                    json: json.clone(),
                }),
                Err(_) => dropped += 1,
            }
        }
    }
    (items, reason, dropped)
}
