//! Strict-mode compatibility check for output schemas (GW-004).
//!
//! OpenAI strict structured outputs require every object to set `additionalProperties: false`
//! and to list every property in `required` (optional values are `["type", "null"]`).

use serde_json::Value;

/// Returns every violation found, with a JSON-pointer-like path, or `Ok(())`.
pub fn check_strict_compatible(schema: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if !is_object_schema(schema) {
        errors.push("#: root schema must be an object".to_owned());
    }
    walk(schema, "#", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn is_object_schema(s: &Value) -> bool {
    match s.get("type") {
        Some(Value::String(t)) => t == "object",
        Some(Value::Array(ts)) => ts.iter().any(|t| t == "object"),
        _ => s.get("properties").is_some(),
    }
}

fn walk(node: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(obj) = node.as_object() else { return };

    if is_object_schema(node) {
        if obj.get("additionalProperties") != Some(&Value::Bool(false)) {
            errors.push(format!("{path}: additionalProperties must be false"));
        }
        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
            let required: Vec<&str> = obj
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            for key in props.keys() {
                if !required.contains(&key.as_str()) {
                    errors.push(format!("{path}/properties/{key}: not listed in required"));
                }
            }
        }
    }

    if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        for (k, v) in props {
            walk(v, &format!("{path}/properties/{k}"), errors);
        }
    }
    for key in ["$defs", "definitions"] {
        if let Some(defs) = obj.get(key).and_then(Value::as_object) {
            for (k, v) in defs {
                walk(v, &format!("{path}/{key}/{k}"), errors);
            }
        }
    }
    for key in ["items", "additionalProperties", "not"] {
        if let Some(v) = obj.get(key) {
            if v.is_object() {
                walk(v, &format!("{path}/{key}"), errors);
            }
        }
    }
    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(arr) = obj.get(key).and_then(Value::as_array) {
            for (i, v) in arr.iter().enumerate() {
                walk(v, &format!("{path}/{key}/{i}"), errors);
            }
        }
    }
}
