//! `input_hash` derivation (PIPE-005): a pure function of a stage's declared inputs (the master
//! plan section 7 cache rule). Wall-clock time and the run id are never inputs.

use std::collections::BTreeMap;

use serde::Serialize;

/// Hex blake3 of `bytes`.
pub fn hash_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Canonical JSON: object keys sorted at every level (independent of serde_json's map order),
/// no whitespace.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let v = serde_json::to_value(value)?;
    let mut out = Vec::new();
    write_canonical(&v, &mut out)?;
    Ok(out)
}

fn write_canonical(v: &serde_json::Value, out: &mut Vec<u8>) -> Result<(), serde_json::Error> {
    match v {
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push(b'{');
            for (i, (k, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend(serde_json::to_vec(k)?);
                out.push(b':');
                write_canonical(item, out)?;
            }
            out.push(b'}');
        }
        other => out.extend(serde_json::to_vec(other)?),
    }
    Ok(())
}

/// Everything a stage's input hash is derived from.
#[derive(Debug, Clone, Default)]
pub struct InputHashParts<'a> {
    pub stage: &'a str,
    pub stage_version: u32,
    /// Upstream stage name to that stage's `output_hash`.
    pub upstream: BTreeMap<String, String>,
    pub config_hash: &'a str,
    /// Model, prompt, analyzer and verification versions the stage uses (name to version).
    pub versions: BTreeMap<String, String>,
}

fn field(h: &mut blake3::Hasher, tag: &str, value: &str) {
    h.update(&(tag.len() as u64).to_le_bytes());
    h.update(tag.as_bytes());
    h.update(&(value.len() as u64).to_le_bytes());
    h.update(value.as_bytes());
}

/// `blake3(stage ‖ stage_version ‖ upstream output hashes sorted by stage ‖ config_hash ‖
/// versions)`, every field length-prefixed so no two inputs can collide by concatenation.
pub fn input_hash(parts: &InputHashParts<'_>) -> String {
    let mut h = blake3::Hasher::new();
    field(&mut h, "stage", parts.stage);
    field(&mut h, "stage_version", &parts.stage_version.to_string());
    for (stage, hash) in &parts.upstream {
        field(&mut h, "upstream", &format!("{stage}={hash}"));
    }
    field(&mut h, "config_hash", parts.config_hash);
    for (name, version) in &parts.versions {
        field(&mut h, "version", &format!("{name}={version}"));
    }
    h.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> InputHashParts<'static> {
        InputHashParts {
            stage: "context",
            stage_version: 1,
            upstream: BTreeMap::from([("diff".to_owned(), "a".repeat(64))]),
            config_hash: "cfg",
            versions: BTreeMap::from([("prompt".to_owned(), "v1".to_owned())]),
        }
    }

    #[test]
    fn deterministic_and_hex() {
        let a = input_hash(&parts());
        assert_eq!(a, input_hash(&parts()));
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn input_hash_changes_with_upstream_hash_and_versions() {
        let base = input_hash(&parts());
        let mut p = parts();
        p.upstream.insert("diff".into(), "b".repeat(64));
        assert_ne!(base, input_hash(&p));
        let mut p = parts();
        p.versions.insert("prompt".into(), "v2".into());
        assert_ne!(base, input_hash(&p));
        let mut p = parts();
        p.stage_version = 2;
        assert_ne!(base, input_hash(&p));
        let mut p = parts();
        p.config_hash = "other";
        assert_ne!(base, input_hash(&p));
    }

    #[test]
    fn input_hash_excludes_run_id_and_time() {
        // The parts struct has no run id or clock field; two builds are identical.
        assert_eq!(input_hash(&parts()), input_hash(&parts()));
    }

    #[test]
    fn concatenation_cannot_collide() {
        let mut a = parts();
        a.upstream = BTreeMap::from([("ab".to_owned(), "c".to_owned())]);
        let mut b = parts();
        b.upstream = BTreeMap::from([("a".to_owned(), "bc".to_owned())]);
        assert_ne!(input_hash(&a), input_hash(&b));
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let v = serde_json::json!({"b": 1, "a": 2});
        assert_eq!(canonical_json(&v).unwrap(), br#"{"a":2,"b":1}"#);
    }
}
