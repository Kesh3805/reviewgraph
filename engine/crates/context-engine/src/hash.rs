//! Deterministic `input_hash` and `package_hash` (CTX-009).

use review_core::reviewer_type::ReviewerType;
use serde_json::Value;

use crate::model::{ContextBudget, ContextPackage};

/// Bumped whenever selection, ranking or compression behaviour changes.
pub const CONTEXT_ENGINE_VERSION: &str = "1";

/// Canonical JSON: object keys sorted, non-integer numbers with 6 decimals, no whitespace.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_f64() {
                match n.as_f64() {
                    Some(f) => out.push_str(&format!("{f:.6}")),
                    None => out.push_str("null"),
                }
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::String(s) => out.push_str(&Value::String(s.clone()).to_string()),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (i, (k, v)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(v, out);
            }
            out.push('}');
        }
    }
}

/// Hex blake3 of the canonical package JSON with `package_hash` blanked.
pub fn package_hash(package: &ContextPackage) -> String {
    let mut copy = package.clone();
    copy.package_hash = String::new();
    let value = serde_json::to_value(&copy).unwrap_or(Value::Null);
    blake3::hash(canonical_json(&value).as_bytes())
        .to_hex()
        .to_string()
}

/// Seals a package: computes and stores its hash.
pub fn seal(mut package: ContextPackage) -> ContextPackage {
    package.package_hash = package_hash(&package);
    package
}

/// Everything that determines a context package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputHashParts<'a> {
    pub change_model_hash: &'a str,
    pub impact_hash: &'a str,
    pub risk_hash: &'a str,
    pub cluster_id: &'a str,
    pub reviewer: ReviewerType,
    pub budget: ContextBudget,
    pub weights_version: &'a str,
    pub profile_version: &'a str,
    pub config_hash: &'a str,
    pub semantic_space: Option<&'a str>,
}

/// `blake3` over length-prefixed fields so no two inputs can alias.
pub fn input_hash(p: &InputHashParts<'_>) -> String {
    let mut h = blake3::Hasher::new();
    let mut field = |bytes: &[u8]| {
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    };
    field(p.change_model_hash.as_bytes());
    field(p.impact_hash.as_bytes());
    field(p.risk_hash.as_bytes());
    field(p.cluster_id.as_bytes());
    field(p.reviewer.as_str().as_bytes());
    field(&p.budget.max_tokens.to_le_bytes());
    field(&p.budget.max_items.to_le_bytes());
    field(p.weights_version.as_bytes());
    field(CONTEXT_ENGINE_VERSION.as_bytes());
    field(p.profile_version.as_bytes());
    field(p.config_hash.as_bytes());
    field(p.semantic_space.unwrap_or("none").as_bytes());
    h.finalize().to_hex().to_string()
}
