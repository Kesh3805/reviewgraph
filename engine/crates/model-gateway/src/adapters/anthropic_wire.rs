//! Anthropic Messages API wire types (response side) and request rendering helpers.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::types::{CachePolicy, InputSection, ModelRequest, ReasoningLevel};

/// Name of the forced tool that carries structured output.
pub const TOOL_NAME: &str = "emit_result";

/// The API allows at most four `cache_control` breakpoints per request.
pub const MAX_CACHE_BREAKPOINTS: usize = 4;

#[derive(Debug, Deserialize)]
pub struct MessagesResponse {
    pub model: Option<String>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: WireUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
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
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

/// Renders a section as `<section name="...">{canonical JSON}</section>`.
pub fn render_section(section: &InputSection) -> String {
    let body =
        serde_jcs::to_string(&section.content).unwrap_or_else(|_| section.content.to_string());
    format!("<section name=\"{}\">{body}</section>", section.name)
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

/// Extended-thinking token budget for a reasoning level, capped below `max_tokens`.
pub fn thinking_budget(level: ReasoningLevel, max_tokens: u32) -> Option<u32> {
    let want = match level {
        ReasoningLevel::Off => return None,
        ReasoningLevel::Low => 2_000,
        ReasoningLevel::Medium => 8_000,
        ReasoningLevel::High => 16_000,
    };
    let budget = want.min(max_tokens.saturating_sub(1));
    // The API requires at least 1024 thinking tokens.
    (budget >= 1_024).then_some(budget)
}

/// Builds the Messages API request body.
pub fn build_request(req: &ModelRequest, model: &str) -> Value {
    let cache_on = !matches!(req.cache, CachePolicy::Disabled);

    let mut system_block = json!({ "type": "text", "text": req.input.system.text.as_ref() });
    let mut breakpoints_used = 0;
    if cache_on {
        system_block["cache_control"] = ephemeral();
        breakpoints_used += 1;
    }

    // At most 4 breakpoints in total: the system block takes one, sections get the first three.
    let mut content: Vec<Value> = Vec::new();
    for section in &req.input.sections {
        let mut block = json!({ "type": "text", "text": render_section(section) });
        if cache_on && section.cache_breakpoint && breakpoints_used < MAX_CACHE_BREAKPOINTS {
            block["cache_control"] = ephemeral();
            breakpoints_used += 1;
        }
        content.push(block);
    }

    let messages = vec![json!({ "role": "user", "content": content })];

    let mut body = json!({
        "model": model,
        "max_tokens": req.max_output_tokens,
        "system": [system_block],
        "messages": messages,
    });

    if let Some(schema) = &req.output_schema {
        // Forced tool use is incompatible with extended thinking, so `reasoning` is ignored.
        body["tools"] = json!([{
            "name": TOOL_NAME,
            "description": "Return the result. Call exactly once.",
            "input_schema": schema.schema.as_ref(),
        }]);
        body["tool_choice"] = json!({ "type": "tool", "name": TOOL_NAME });
    } else if let Some(budget) = thinking_budget(req.reasoning, req.max_output_tokens) {
        body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
    }
    body
}
