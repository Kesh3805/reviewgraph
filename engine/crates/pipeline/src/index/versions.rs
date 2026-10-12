//! Snapshot version fields and fingerprint (IDX-002, ADR-015).
//!
//! Everything here is pure: the same inputs always give the same fingerprint, which is what
//! lets a full index short-circuit onto an existing `ready` snapshot.

use std::collections::BTreeMap;

use graph_storage::types::{SnapshotVersions, GRAPH_SCHEMA_VERSION};

/// Per-component hashes of the effective configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigComponents {
    pub source_roots: [u8; 32],
    pub tsconfig: [u8; 32],
    pub generated_globs: [u8; 32],
    pub review_rules: [u8; 32],
}

impl ConfigComponents {
    /// Hash hand-normalized byte strings into components. Callers pass a canonical form
    /// (sorted keys, normalized paths), so whitespace-only edits upstream do not matter.
    pub fn from_canonical(
        source_roots: &[u8],
        tsconfig: &[u8],
        generated_globs: &[u8],
        review_rules: &[u8],
    ) -> Self {
        Self {
            source_roots: *blake3::hash(source_roots).as_bytes(),
            tsconfig: *blake3::hash(tsconfig).as_bytes(),
            generated_globs: *blake3::hash(generated_globs).as_bytes(),
            review_rules: *blake3::hash(review_rules).as_bytes(),
        }
    }

    /// `blake3(source_roots || tsconfig || generated_globs || review_rules)`.
    pub fn config_hash(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(&self.source_roots);
        h.update(&self.tsconfig);
        h.update(&self.generated_globs);
        h.update(&self.review_rules);
        *h.finalize().as_bytes()
    }
}

/// Inputs of [`compute_versions`].
#[derive(Debug, Clone)]
pub struct VersionInputs<'a> {
    pub repository_id: &'a str,
    pub commit_sha: &'a str,
    /// Language analyzer versions plus `"linker"`, keyed by name.
    pub analyzer_versions: BTreeMap<String, String>,
    /// `"tree-sitter"`, `"tree-sitter-typescript"`, ...
    pub parser_versions: BTreeMap<String, String>,
    pub config: ConfigComponents,
    pub profile_version: Option<&'a str>,
}

fn put_str(h: &mut blake3::Hasher, s: &str) {
    h.update(&(s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}

fn put_map(h: &mut blake3::Hasher, m: &BTreeMap<String, String>) {
    h.update(&(m.len() as u64).to_le_bytes());
    for (k, v) in m {
        put_str(h, k);
        put_str(h, v);
    }
}

fn hex32(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Compute the version columns recorded on every snapshot. The fingerprint covers
/// `repository_id || commit_sha || analyzer_versions || graph_schema_version || config_hash ||
/// parser_versions || profile_version`, length-prefixed so no two inputs can collide by
/// concatenation.
pub fn compute_versions(inputs: &VersionInputs<'_>) -> SnapshotVersions {
    let config_hash = inputs.config.config_hash();

    let mut h = blake3::Hasher::new();
    put_str(&mut h, inputs.repository_id);
    put_str(&mut h, inputs.commit_sha);
    put_map(&mut h, &inputs.analyzer_versions);
    h.update(&GRAPH_SCHEMA_VERSION.to_le_bytes());
    h.update(&config_hash);
    put_map(&mut h, &inputs.parser_versions);
    match inputs.profile_version {
        Some(p) => {
            h.update(&[1]);
            put_str(&mut h, p);
        }
        None => {
            h.update(&[0]);
        }
    }
    let fingerprint = *h.finalize().as_bytes();

    let mut components = BTreeMap::new();
    components.insert(
        "source_roots".to_owned(),
        hex32(&inputs.config.source_roots),
    );
    components.insert("tsconfig".to_owned(), hex32(&inputs.config.tsconfig));
    components.insert(
        "generated_globs".to_owned(),
        hex32(&inputs.config.generated_globs),
    );
    components.insert(
        "review_rules".to_owned(),
        hex32(&inputs.config.review_rules),
    );

    SnapshotVersions {
        graph_schema_version: GRAPH_SCHEMA_VERSION,
        analyzer_versions: inputs.analyzer_versions.clone(),
        config_hash,
        config_components: components,
        fingerprint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn base() -> VersionInputs<'static> {
        VersionInputs {
            repository_id: "repo-1",
            commit_sha: "aaaa",
            analyzer_versions: map(&[("typescript", "1.0.0"), ("linker", "1")]),
            parser_versions: map(&[("tree-sitter", "0.25")]),
            config: ConfigComponents::from_canonical(b"roots", b"ts", b"gen", b"rules"),
            profile_version: Some("p1"),
        }
    }

    #[test]
    fn fingerprint_is_stable_for_same_inputs() {
        assert_eq!(
            compute_versions(&base()).fingerprint,
            compute_versions(&base()).fingerprint
        );
    }

    #[test]
    fn fingerprint_changes_with_each_input() {
        let f0 = compute_versions(&base()).fingerprint;
        let mut variants = Vec::new();

        let mut v = base();
        v.commit_sha = "bbbb";
        variants.push(v);
        let mut v = base();
        v.repository_id = "repo-2";
        variants.push(v);
        let mut v = base();
        v.analyzer_versions
            .insert("typescript".into(), "1.0.1".into());
        variants.push(v);
        let mut v = base();
        v.parser_versions
            .insert("tree-sitter".into(), "0.26".into());
        variants.push(v);
        let mut v = base();
        v.profile_version = None;
        variants.push(v);
        let mut v = base();
        v.config = ConfigComponents::from_canonical(b"roots", b"ts2", b"gen", b"rules");
        variants.push(v);

        for v in variants {
            assert_ne!(compute_versions(&v).fingerprint, f0);
        }
    }

    #[test]
    fn only_the_changed_component_differs() {
        let a = compute_versions(&base());
        let mut b = base();
        b.config = ConfigComponents::from_canonical(b"roots", b"ts2", b"gen", b"rules");
        let b = compute_versions(&b);
        assert_ne!(a.config_hash, b.config_hash);
        assert_ne!(
            a.config_components["tsconfig"],
            b.config_components["tsconfig"]
        );
        assert_eq!(
            a.config_components["source_roots"],
            b.config_components["source_roots"]
        );
        assert_eq!(
            a.config_components["generated_globs"],
            b.config_components["generated_globs"]
        );
    }

    #[test]
    fn records_schema_and_linker_versions() {
        let v = compute_versions(&base());
        assert_eq!(v.graph_schema_version, GRAPH_SCHEMA_VERSION);
        assert_eq!(
            v.analyzer_versions.get("linker").map(String::as_str),
            Some("1")
        );
        assert_ne!(v.fingerprint, [0u8; 32]);
    }
}
