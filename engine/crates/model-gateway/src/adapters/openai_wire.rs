//! OpenAI Responses API wire types (response side) and request rendering.

use serde::Deserialize;
use serde_json::{json, Value};

use super::anthropic_wire::render_section;
use crate::types::{ModelRequest, ReasoningLevel, RouteCandidate};

#[derive(Debug, Deserialize)]
pub struct ResponsesResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    pub status: Option<String>,
    pub incomplete_details: Option<IncompleteDetails>,
    #[serde(default)]
    pub output: Vec<OutputItem>,
    #[serde(default)]
    pub usage: WireUsage,
}

#[derive(Debug, Deserialize)]
pub struct IncompleteDetails {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message {
        #[serde(default)]
        content: Vec<ContentPart>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    OutputText {
        text: String,
    },
    Refusal {
        refusal: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Default, Deserialize)]
pub struct WireUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub input_tokens_details: InputDetails,
    #[serde(default)]
    pub output_tokens_details: OutputDetails,
}

#[derive(Debug, Default, Deserialize)]
pub struct InputDetails {
    #[serde(default)]
    pub cached_tokens: u32,
}

#[derive(Debug, Default, Deserialize)]
pub struct OutputDetails {
    #[serde(default)]
    pub reasoning_tokens: u32,
}

/// Schema names must match `^[a-zA-Z0-9_-]{1,64}$`.
fn schema_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "result".to_owned()
    } else {
        cleaned
    }
}

fn effort(level: ReasoningLevel) -> Option<&'static str> {
    match level {
        ReasoningLevel::Off => None,
        ReasoningLevel::Low => Some("low"),
        ReasoningLevel::Medium => Some("medium"),
        ReasoningLevel::High => Some("high"),
    }
}

/// Builds the Responses API request body. `store: false` is always sent.
pub fn build_request(req: &ModelRequest, candidate: &RouteCandidate) -> Value {
    // Stable sections come first in the input, so OpenAI's automatic prefix caching can hit.
    let text = req
        .input
        .sections
        .iter()
        .map(render_section)
        .collect::<Vec<_>>()
        .join("\n");
    let mut body = json!({
        "model": candidate.model,
        "instructions": req.input.system.text.as_ref(),
        "input": [{
            "role": "user",
            "content": [{ "type": "input_text", "text": text }],
        }],
        "max_output_tokens": req.max_output_tokens,
        "store": false,
    });
    if let Some(schema) = &req.output_schema {
        body["text"] = json!({
            "format": {
                "type": "json_schema",
                "name": schema_name(&schema.name),
                "schema": schema.schema.as_ref(),
                "strict": true,
            }
        });
    }
    if candidate.supports_reasoning {
        if let Some(e) = effort(req.reasoning) {
            body["reasoning"] = json!({ "effort": e });
        }
    }
    body
}
