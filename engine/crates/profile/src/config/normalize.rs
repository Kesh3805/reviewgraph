//! The normalized form of a config and its `config_hash` (POL-001).
//!
//! Normalization serializes the typed config (so every default is expanded) and sorts map keys;
//! the hash is `blake3(canonical_json(normalized))`, so key order, comments and whitespace in the
//! YAML never change it.

use repository::config_hash::canonical;
use review_core::version::ConfigHash;
use serde_json::Value;

use super::schema::ReviewConfigV1;

/// Defaults expanded, map keys sorted.
pub fn normalized(config: &ReviewConfigV1) -> Value {
    canonical(&serde_json::to_value(config).unwrap_or(Value::Null))
}

/// Compact canonical JSON bytes of a normalized value.
pub fn canonical_bytes(normalized: &Value) -> Vec<u8> {
    serde_json::to_vec(&canonical(normalized)).unwrap_or_default()
}

/// `blake3(canonical_json(normalized))`.
pub fn config_hash(config: &ReviewConfigV1) -> ConfigHash {
    ConfigHash::of(&canonical_bytes(&normalized(config)))
}

/// The hash of an invalid file. It must not equal the hash of the defaults (the stored
/// validation errors would otherwise collide with a repository that has no config) nor depend
/// on anything but the file's bytes.
pub fn invalid_config_hash(defaults: &ReviewConfigV1, raw: &[u8]) -> ConfigHash {
    let mut bytes = canonical_bytes(&normalized(defaults));
    bytes.extend_from_slice(b"\0invalid\0");
    bytes.extend_from_slice(blake3::hash(raw).to_hex().as_bytes());
    ConfigHash::of(&bytes)
}

/// The hash of only the keys that change the graph: ignore globs, generated-code globs and
/// index tolerance. A change here forces a full rebuild (INC-011); rule-only changes do not.
pub fn graph_inputs_hash(config: &ReviewConfigV1) -> [u8; 32] {
    let value = serde_json::json!({
        "ignore": config.ignore,
        "generated": config.generated,
        "review_generated_ignore": config.review.generated.ignore,
        "index": config.index,
    });
    *blake3::hash(&canonical_bytes(&value)).as_bytes()
}
