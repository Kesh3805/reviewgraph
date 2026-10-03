//! Canonical request hash (GW-001).
//!
//! `blake3(JCS({v:1, task, tier, reasoning, prompt_id, prompt_version, prompt_sha, sections,
//! output_schema_hash, max_output_tokens, repair}))`. It is independent of provider and model.
//! The gateway computes it after pre-send redaction, so it describes what leaves the process.

use serde_json::{json, Value};

use crate::types::{ModelRequest, RequestHash};

/// blake3 over the JCS encoding of `value`, lowercase hex.
pub fn hash_value(value: &Value) -> String {
    // JCS of a `Value` cannot fail; fall back to the compact form to stay panic free.
    let bytes = serde_jcs::to_vec(value).unwrap_or_else(|_| value.to_string().into_bytes());
    blake3::hash(&bytes).to_hex().to_string()
}

fn wire_name<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Computes the request hash.
pub fn request_hash(req: &ModelRequest) -> RequestHash {
    let sections: Vec<Value> = req
        .input
        .sections
        .iter()
        .map(|s| json!({ "name": s.name, "content": s.content }))
        .collect();
    let repair = match &req.input.repair {
        Some(r) => json!({ "previous_output": r.previous_output, "errors": r.errors }),
        None => Value::Null,
    };
    let canonical = json!({
        "v": 1,
        "task": wire_name(&req.task),
        "tier": wire_name(&req.tier),
        "reasoning": wire_name(&req.reasoning),
        "prompt_id": req.input.system.prompt_id,
        "prompt_version": req.input.system.prompt_version,
        "prompt_sha": req.input.system.prompt_sha,
        "sections": sections,
        "output_schema_hash": req.output_schema.as_ref().map(|s| s.hash.clone()),
        "max_output_tokens": req.max_output_tokens,
        "repair": repair,
    });
    RequestHash(hash_value(&canonical))
}
