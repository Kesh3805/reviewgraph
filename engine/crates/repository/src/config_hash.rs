//! `config_hash`: a digest of every configuration input that changes derived intelligence
//! (INIT-012). Formatting and comments never change it; a base tsconfig change does, because the
//! effective (post-merge) options are hashed.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::error::InitWarning;
use crate::manifests::{Ecosystem, ManifestFacts};
use crate::read::BoundedReader;
use crate::review_dir::read_config_raw;
use crate::tsconfig::TsConfigSet;
use crate::walk::FileInventory;

/// Sorted-key copy of a JSON value, so serialization is canonical whatever the map backend.
pub fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> =
                map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn canonical_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&canonical(value)).unwrap_or_default()
}

fn part(hasher: &mut blake3::Hasher, tag: &str, bytes: &[u8]) {
    hasher.update(&(tag.len() as u64).to_le_bytes());
    hasher.update(tag.as_bytes());
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn yaml_to_json(text: &str) -> Option<Value> {
    serde_yaml::from_str::<Value>(text).ok()
}

/// Computes the config hash.
pub fn compute_config_hash(
    root: &Path,
    inventory: &FileInventory,
    reader: &BoundedReader,
    tsconfigs: &TsConfigSet,
    manifests: &ManifestFacts,
) -> ([u8; 32], Vec<InitWarning>) {
    let mut warnings = Vec::new();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"rg.config.v1\0");

    // (a) .review/config.yaml as canonical JSON: comments and formatting are ignored.
    match read_config_raw(root) {
        None => part(&mut hasher, "review_config", b"none"),
        Some(raw) => {
            let parsed = std::str::from_utf8(&raw).ok().and_then(yaml_to_json);
            match parsed {
                Some(value) => part(&mut hasher, "review_config", &canonical_bytes(&value)),
                None => {
                    warnings.push(InitWarning::new(
                        "config_yaml_parse",
                        None,
                        ".review/config.yaml could not be parsed; its raw bytes were hashed",
                    ));
                    part(&mut hasher, "review_config_raw", &raw);
                }
            }
        }
    }

    // (b) effective tsconfig options, sorted by path.
    for config in &tsconfigs.configs {
        let value = serde_json::to_value(&config.effective).unwrap_or(Value::Null);
        part(&mut hasher, config.path.as_str(), &canonical_bytes(&value));
    }

    // (c) .reviewignore files, raw.
    for entry in inventory
        .entries
        .iter()
        .filter(|e| e.path.as_str().rsplit('/').next() == Some(".reviewignore"))
    {
        let bytes = reader
            .read_prefix(entry, usize::try_from(entry.size).unwrap_or(usize::MAX))
            .unwrap_or_default();
        part(&mut hasher, entry.path.as_str(), &bytes);
    }

    // (d) workspace definition sources.
    if let Some(entry) = inventory.find("pnpm-workspace.yaml") {
        if let Ok(text) = reader.read_text(entry, 256 * 1024) {
            let bytes = yaml_to_json(&text)
                .map(|v| canonical_bytes(&v))
                .unwrap_or_else(|| text.into_bytes());
            part(&mut hasher, "pnpm-workspace.yaml", &bytes);
        }
    }
    if let Some(root_manifest) = manifests
        .manifests
        .iter()
        .find(|m| m.ecosystem == Ecosystem::Npm && m.dir().is_root())
    {
        if let Some(ws) = root_manifest
            .npm
            .as_ref()
            .and_then(|n| n.workspaces.as_ref())
        {
            let value = serde_json::to_value(ws).unwrap_or(Value::Null);
            part(
                &mut hasher,
                "package.json#workspaces",
                &canonical_bytes(&value),
            );
        }
    }
    if let Some(entry) = inventory.find("nx.json") {
        if let Ok(text) = reader.read_text(entry, 256 * 1024) {
            let bytes = crate::jsonc::parse_jsonc(text.as_bytes())
                .map(|v| canonical_bytes(&v))
                .unwrap_or_else(|_| text.into_bytes());
            part(&mut hasher, "nx.json", &bytes);
        }
    }
    if let Some(entry) = inventory.find("Cargo.toml") {
        if let Ok(text) = reader.read_text(entry, 256 * 1024) {
            if let Ok(table) = text.parse::<toml::Table>() {
                if let Some(workspace) = table.get("workspace") {
                    let value = serde_json::to_value(workspace).unwrap_or(Value::Null);
                    part(
                        &mut hasher,
                        "Cargo.toml#workspace",
                        &canonical_bytes(&value),
                    );
                }
            }
        }
    }

    (*hasher.finalize().as_bytes(), warnings)
}
