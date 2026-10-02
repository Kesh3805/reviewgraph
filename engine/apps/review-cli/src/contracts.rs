//! Contract registry and the `review contracts export` implementation (FND-007).
//!
//! Every Rust type shared with TypeScript is registered here. Export is deterministic: keys are
//! sorted, there are no timestamps and files are newline-terminated.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use schemars::schema::RootSchema;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// One exported contract type.
#[derive(Debug, Clone, Copy)]
pub struct ContractType {
    pub name: &'static str,
    pub schema: fn() -> RootSchema,
}

fn schema_of<T: schemars::JsonSchema>() -> RootSchema {
    schemars::gen::SchemaGenerator::default().into_root_schema_for::<T>()
}

fn entry<T: schemars::JsonSchema>(name: &'static str) -> ContractType {
    ContractType {
        name,
        schema: schema_of::<T>,
    }
}

/// All contract types, in registration order. Domain tasks register their own types here.
pub fn registry() -> Vec<ContractType> {
    use review_core::contracts::SchemaInfo;
    use review_core::finding::{
        CandidateFinding, FindingCategory, FindingState, PublishedFinding, ReviewerType, Severity,
        VerifiedFinding,
    };
    use review_core::publication::{CheckConclusion, ReviewEvent};
    use review_core::review::{ReviewState, ReviewTrigger, ReviewerRunState};
    use review_core::ErrorClass;
    vec![
        entry::<SchemaInfo>("SchemaInfo"),
        entry::<ErrorClass>("ErrorClass"),
        entry::<ReviewEvent>("ReviewEvent"),
        entry::<CheckConclusion>("CheckConclusion"),
        entry::<ReviewState>("ReviewState"),
        entry::<ReviewerRunState>("ReviewerRunState"),
        entry::<ReviewTrigger>("ReviewTrigger"),
        entry::<FindingState>("FindingState"),
        entry::<Severity>("Severity"),
        entry::<FindingCategory>("FindingCategory"),
        entry::<ReviewerType>("ReviewerType"),
        entry::<CandidateFinding>("CandidateFinding"),
        entry::<VerifiedFinding>("VerifiedFinding"),
        entry::<PublishedFinding>("PublishedFinding"),
    ]
}

/// Recursively sort object keys so output does not depend on map iteration order.
fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> =
                map.into_iter().map(|(k, v)| (k, sort_keys(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

fn render(value: &Value) -> anyhow::Result<String> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

fn schema_document(contract: &ContractType) -> anyhow::Result<String> {
    let value = serde_json::to_value((contract.schema)())
        .with_context(|| format!("schema for {} is not serializable", contract.name))?;
    let mut value = sort_keys(value);
    if let Value::Object(map) = &mut value {
        map.insert(
            "$id".to_owned(),
            Value::String(format!("urn:reviewgraph:contracts:{}", contract.name)),
        );
    }
    render(&sort_keys(value))
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Export `contracts` into `out_dir`. Returns the written paths (schemas then `index.json`).
pub fn export_types(contracts: &[ContractType], out_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    for c in contracts {
        if !seen.insert(c.name) {
            bail!("duplicate contract name: {}", c.name);
        }
    }

    // Render everything first so a failure writes nothing.
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    for c in contracts {
        let doc = schema_document(c)?;
        hashes.insert(c.name.to_owned(), sha256_hex(&doc));
        files.insert(format!("{}.schema.json", c.name), doc);
    }
    let index = serde_json::json!({
        "contracts_version": review_core::contracts::CONTRACTS_VERSION,
        "types": seen.iter().collect::<Vec<_>>(),
        "sha256": hashes,
    });
    files.insert("index.json".to_owned(), render(&sort_keys(index))?);

    fs::create_dir_all(out_dir).with_context(|| format!("cannot create {}", out_dir.display()))?;

    // Stage into a sibling directory, then rename each file into place.
    let staging = staging_dir(out_dir);
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging).with_context(|| format!("cannot create {}", staging.display()))?;
    let result = (|| -> anyhow::Result<Vec<PathBuf>> {
        for (name, text) in &files {
            fs::write(staging.join(name), text).with_context(|| format!("cannot write {name}"))?;
        }
        let mut written = Vec::new();
        for name in files.keys() {
            let dest = out_dir.join(name);
            fs::rename(staging.join(name), &dest)
                .with_context(|| format!("cannot move {name} into {}", out_dir.display()))?;
            written.push(dest);
        }
        // Remove schemas that are no longer registered.
        for entry in fs::read_dir(out_dir)? {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name.ends_with(".schema.json") && !files.contains_key(&file_name) {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(written)
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

fn staging_dir(out_dir: &Path) -> PathBuf {
    let mut name = out_dir
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".tmp-{}", std::process::id()));
    out_dir.with_file_name(name)
}

/// Export the full registry.
pub fn export(out_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    export_types(&registry(), out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn export_is_deterministic() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        export(a.path()).unwrap();
        export(b.path()).unwrap();
        assert_eq!(read_all(a.path()), read_all(b.path()));
        // Re-exporting into the same directory is also a no-op.
        export(a.path()).unwrap();
        assert_eq!(read_all(a.path()), read_all(b.path()));
    }

    #[test]
    fn export_removes_stale_schemas() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Old.schema.json"), "{}").unwrap();
        fs::write(dir.path().join("keep.txt"), "x").unwrap();
        export(dir.path()).unwrap();
        assert!(!dir.path().join("Old.schema.json").exists());
        assert!(dir.path().join("keep.txt").exists());
        assert!(dir.path().join("SchemaInfo.schema.json").exists());
        assert!(dir.path().join("index.json").exists());
    }

    #[test]
    fn registry_names_unique() {
        let names: Vec<_> = registry().iter().map(|c| c.name).collect();
        let unique: BTreeSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len());
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let c = registry()[0];
        let dir = tempfile::tempdir().unwrap();
        assert!(export_types(&[c, c], dir.path()).is_err());
    }

    #[test]
    fn schema_info_schema_has_no_additional_properties() {
        let dir = tempfile::tempdir().unwrap();
        export(dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join("SchemaInfo.schema.json")).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["additionalProperties"], Value::Bool(false));
        assert_eq!(v["$id"], "urn:reviewgraph:contracts:SchemaInfo");
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn contracts_export_contains_finding_state_enum_values() {
        let dir = tempfile::tempdir().unwrap();
        export(dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join("FindingState.schema.json")).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let exported: Vec<&str> = v["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        let expected: Vec<&str> = review_core::finding::FindingState::ALL
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(exported, expected);
    }

    #[test]
    fn contracts_export_contains_review_state_values() {
        let dir = tempfile::tempdir().unwrap();
        export(dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join("ReviewState.schema.json")).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let exported: Vec<&str> = v["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        let expected: Vec<&str> = review_core::review::ReviewState::ALL
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(exported, expected);
        assert_eq!(exported.len(), 13);
    }
}
