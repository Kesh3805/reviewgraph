//! Change-model shell entities (DOM-005).
//!
//! Each type names the task that populates it:
//! - [`Hunk`] and [`ChangedFile`]: DIFF-001 (line content and the full hunk model come later).
//! - [`ChangedSymbol`] and [`SymbolChange`]: CHG-001 (change classes arrive with CHG-002).
//! - [`ChangeCluster`] and [`ChangeClusterKey`]: IMP-009 (the clustering algorithm).

use std::fmt;
use std::str::FromStr;

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::ids::{SymbolId, SymbolKey};
use crate::location::{DiffSide, RepoPath, SourceRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

/// Unified-diff hunk header: `@@ -old_start,old_lines +new_start,new_lines @@`.
/// Populated by DIFF-001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

/// A file touched by a pull request. Populated by DIFF-001.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedFile {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub status: FileChangeStatus,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

impl ChangedFile {
    /// Validates the invariants:
    /// - `old_path` is present exactly when the status is `Renamed` or `Copied`;
    /// - `Added` has no hunk with `old_lines > 0`;
    /// - `Deleted` has no hunk with `new_lines > 0`;
    /// - a binary file has no hunks.
    pub fn new(
        path: RepoPath,
        old_path: Option<RepoPath>,
        status: FileChangeStatus,
        binary: bool,
        hunks: Vec<Hunk>,
    ) -> Result<Self, CoreError> {
        let out_of_range = |field: &'static str, value: &str| CoreError::OutOfRange {
            field,
            value: value.to_owned(),
        };
        let needs_old = matches!(status, FileChangeStatus::Renamed | FileChangeStatus::Copied);
        if needs_old != old_path.is_some() {
            return Err(out_of_range(
                "changed_file.old_path",
                if needs_old {
                    "missing for a renamed or copied file"
                } else {
                    "present for a file that is not renamed or copied"
                },
            ));
        }
        if status == FileChangeStatus::Added && hunks.iter().any(|h| h.old_lines > 0) {
            return Err(out_of_range(
                "changed_file.hunks",
                "an added file cannot have old-side lines",
            ));
        }
        if status == FileChangeStatus::Deleted && hunks.iter().any(|h| h.new_lines > 0) {
            return Err(out_of_range(
                "changed_file.hunks",
                "a deleted file cannot have new-side lines",
            ));
        }
        if binary && !hunks.is_empty() {
            return Err(out_of_range(
                "changed_file.hunks",
                "a binary file has no hunks",
            ));
        }
        Ok(Self {
            path,
            old_path,
            status,
            binary,
            hunks,
        })
    }
}

impl<'de> Deserialize<'de> for ChangedFile {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            path: RepoPath,
            old_path: Option<RepoPath>,
            status: FileChangeStatus,
            binary: bool,
            hunks: Vec<Hunk>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self::new(r.path, r.old_path, r.status, r.binary, r.hunks).map_err(serde::de::Error::custom)
    }
}

/// How a symbol changed. Populated by CHG-001; the finer change classes are CHG-002.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SymbolChange {
    Added,
    Removed,
    Modified {
        signature: bool,
        body: bool,
        attrs: bool,
    },
    Renamed {
        from: SymbolId,
        similarity: f32,
    },
}

impl SymbolChange {
    /// A modification; at least one flag must be set.
    pub fn modified(signature: bool, body: bool, attrs: bool) -> Result<Self, CoreError> {
        Self::Modified {
            signature,
            body,
            attrs,
        }
        .validated()
    }

    /// A rename; `similarity` must be finite and within `[0, 1]`.
    pub fn renamed(from: SymbolId, similarity: f32) -> Result<Self, CoreError> {
        Self::Renamed { from, similarity }.validated()
    }

    fn validated(self) -> Result<Self, CoreError> {
        match &self {
            Self::Modified {
                signature: false,
                body: false,
                attrs: false,
            } => Err(CoreError::OutOfRange {
                field: "symbol_change.modified",
                value: "at least one of signature, body, attrs must be true".to_owned(),
            }),
            Self::Renamed { similarity, .. }
                if !similarity.is_finite() || !(0.0..=1.0).contains(similarity) =>
            {
                Err(CoreError::OutOfRange {
                    field: "symbol_change.similarity",
                    value: similarity.to_string(),
                })
            }
            _ => Ok(self),
        }
    }
}

impl<'de> Deserialize<'de> for SymbolChange {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Raw {
            Added,
            Removed,
            Modified {
                signature: bool,
                body: bool,
                attrs: bool,
            },
            Renamed {
                from: SymbolId,
                similarity: f32,
            },
        }
        let change = match Raw::deserialize(deserializer)? {
            Raw::Added => Self::Added,
            Raw::Removed => Self::Removed,
            Raw::Modified {
                signature,
                body,
                attrs,
            } => Self::Modified {
                signature,
                body,
                attrs,
            },
            Raw::Renamed { from, similarity } => Self::Renamed { from, similarity },
        };
        change.validated().map_err(serde::de::Error::custom)
    }
}

/// A symbol touched by a pull request. Populated by CHG-001.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedSymbol {
    pub symbol_key: SymbolKey,
    pub symbol_id: SymbolId,
    pub path: RepoPath,
    pub side: DiffSide,
    pub range: SourceRange,
    pub change: SymbolChange,
}

/// Order-independent identity of a change cluster: blake3 over the sorted, deduplicated member
/// keys, truncated to 16 bytes. Wire form: 32 lowercase hex characters. Because it is
/// deterministic, `reviewer_runs` is unique on `(review_run_id, reviewer, cluster_key)` and a
/// retried review stage maps to the same rows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChangeClusterKey([u8; 16]);

impl ChangeClusterKey {
    pub fn of(members: &[SymbolKey]) -> Self {
        let mut sorted: Vec<SymbolKey> = members.to_vec();
        sorted.sort();
        sorted.dedup();
        let mut hasher = blake3::Hasher::new();
        for key in &sorted {
            hasher.update(key.as_bytes());
        }
        let mut out = [0u8; 16];
        out.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        Self(out)
    }

    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for ChangeClusterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for ChangeClusterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChangeClusterKey({self})")
    }
}

impl FromStr for ChangeClusterKey {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: String| CoreError::InvalidId {
            kind: "ChangeClusterKey",
            reason,
        };
        if s.len() != 32 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(invalid("expected 32 lowercase hex characters".to_owned()));
        }
        let mut out = [0u8; 16];
        hex::decode_to_slice(s, &mut out).map_err(|e| invalid(e.to_string()))?;
        Ok(Self(out))
    }
}

impl Serialize for ChangeClusterKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ChangeClusterKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ChangeClusterKey {
    fn schema_name() -> String {
        "ChangeClusterKey".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("ChangeClusterKey", "^[0-9a-f]{32}$")
    }
}

/// A group of related changed symbols reviewed together. Populated by IMP-009.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeCluster {
    pub key: ChangeClusterKey,
    pub members: Vec<SymbolKey>,
    pub module: Option<String>,
}

impl ChangeCluster {
    /// Stores the members sorted and deduplicated and derives the key from them.
    pub fn new(mut members: Vec<SymbolKey>, module: Option<String>) -> Self {
        members.sort();
        members.dedup();
        Self {
            key: ChangeClusterKey::of(&members),
            members,
            module,
        }
    }
}

impl<'de> Deserialize<'de> for ChangeCluster {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            key: ChangeClusterKey,
            members: Vec<SymbolKey>,
            module: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let cluster = Self::new(raw.members.clone(), raw.module);
        if cluster.members != raw.members || cluster.key != raw.key {
            return Err(serde::de::Error::custom(
                "cluster members must be sorted and unique, and the key must match them",
            ));
        }
        Ok(cluster)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn p(s: &str) -> RepoPath {
        RepoPath::new(s).unwrap()
    }

    fn hunk(old_lines: u32, new_lines: u32) -> Hunk {
        Hunk {
            old_start: 1,
            old_lines,
            new_start: 1,
            new_lines,
        }
    }

    #[test]
    fn renamed_requires_old_path() {
        for status in [FileChangeStatus::Renamed, FileChangeStatus::Copied] {
            assert!(ChangedFile::new(p("b.ts"), None, status, false, vec![]).is_err());
            assert!(ChangedFile::new(p("b.ts"), Some(p("a.ts")), status, false, vec![]).is_ok());
        }
    }

    #[test]
    fn modified_forbids_old_path() {
        for status in [
            FileChangeStatus::Added,
            FileChangeStatus::Modified,
            FileChangeStatus::Deleted,
        ] {
            assert!(ChangedFile::new(p("b.ts"), Some(p("a.ts")), status, false, vec![]).is_err());
            assert!(ChangedFile::new(p("b.ts"), None, status, false, vec![]).is_ok());
        }
    }

    #[test]
    fn added_file_has_no_old_side_lines() {
        let added = FileChangeStatus::Added;
        assert!(ChangedFile::new(p("a.ts"), None, added, false, vec![hunk(0, 5)]).is_ok());
        assert!(ChangedFile::new(p("a.ts"), None, added, false, vec![hunk(1, 5)]).is_err());
    }

    #[test]
    fn deleted_file_has_no_new_side_lines() {
        let deleted = FileChangeStatus::Deleted;
        assert!(ChangedFile::new(p("a.ts"), None, deleted, false, vec![hunk(5, 0)]).is_ok());
        assert!(ChangedFile::new(p("a.ts"), None, deleted, false, vec![hunk(5, 1)]).is_err());
    }

    #[test]
    fn binary_has_no_hunks() {
        let m = FileChangeStatus::Modified;
        assert!(ChangedFile::new(p("a.png"), None, m, true, vec![]).is_ok());
        assert!(ChangedFile::new(p("a.png"), None, m, true, vec![hunk(1, 1)]).is_err());
    }

    #[test]
    fn changed_file_deserialize_validates() {
        let bad = r#"{"path":"a.ts","old_path":null,"status":"renamed","binary":false,"hunks":[]}"#;
        assert!(serde_json::from_str::<ChangedFile>(bad).is_err());
        let good =
            r#"{"path":"b.ts","old_path":"a.ts","status":"renamed","binary":false,"hunks":[]}"#;
        assert!(serde_json::from_str::<ChangedFile>(good).is_ok());
        let traversal =
            r#"{"path":"../b.ts","old_path":null,"status":"added","binary":false,"hunks":[]}"#;
        assert!(serde_json::from_str::<ChangedFile>(traversal).is_err());
    }

    #[test]
    fn symbol_modified_requires_a_flag() {
        assert!(SymbolChange::modified(false, false, false).is_err());
        assert!(SymbolChange::modified(false, true, false).is_ok());
        let none = r#"{"kind":"modified","signature":false,"body":false,"attrs":false}"#;
        assert!(serde_json::from_str::<SymbolChange>(none).is_err());
        let body = r#"{"kind":"modified","signature":false,"body":true,"attrs":false}"#;
        assert!(serde_json::from_str::<SymbolChange>(body).is_ok());
    }

    #[test]
    fn rename_similarity_bounds() {
        let from = || SymbolId::from_canonical_unchecked("ts:a#f/function");
        assert!(SymbolChange::renamed(from(), 0.0).is_ok());
        assert!(SymbolChange::renamed(from(), 1.0).is_ok());
        assert!(SymbolChange::renamed(from(), 0.85).is_ok());
        for bad in [-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(SymbolChange::renamed(from(), bad).is_err(), "{bad}");
        }
        let json = r#"{"kind":"renamed","from":"ts:a#f/function","similarity":1.5}"#;
        assert!(serde_json::from_str::<SymbolChange>(json).is_err());
    }

    fn key(n: u8) -> SymbolKey {
        SymbolKey::from_bytes([n; 16])
    }

    #[test]
    fn cluster_key_dedupes() {
        let a = ChangeClusterKey::of(&[key(1), key(2)]);
        assert_eq!(a, ChangeClusterKey::of(&[key(2), key(1), key(2), key(1)]));
        assert_ne!(a, ChangeClusterKey::of(&[key(1)]));
        let cluster = ChangeCluster::new(vec![key(3), key(1), key(3)], Some("auth".into()));
        assert_eq!(cluster.members, vec![key(1), key(3)]);
        assert_eq!(cluster.key, ChangeClusterKey::of(&[key(1), key(3)]));
    }

    #[test]
    fn cluster_wire_form_roundtrips_and_validates() {
        let cluster = ChangeCluster::new(vec![key(2), key(1)], None);
        let json = serde_json::to_string(&cluster).unwrap();
        assert_eq!(
            serde_json::from_str::<ChangeCluster>(&json).unwrap(),
            cluster
        );
        assert_eq!(
            cluster.key.to_string().parse::<ChangeClusterKey>().unwrap(),
            cluster.key
        );
        let unsorted = format!(
            r#"{{"key":"{}","members":["{}","{}"],"module":null}}"#,
            cluster.key,
            key(2),
            key(1)
        );
        assert!(serde_json::from_str::<ChangeCluster>(&unsorted).is_err());
    }

    proptest! {
        #[test]
        fn cluster_key_order_independent(
            raw in prop::collection::vec(any::<[u8; 16]>(), 0..12),
            seed in any::<u64>(),
        ) {
            let members: Vec<SymbolKey> = raw.into_iter().map(SymbolKey::from_bytes).collect();
            let mut shuffled = members.clone();
            // Deterministic pseudo-shuffle driven by the proptest seed.
            let mut state = seed | 1;
            for i in (1..shuffled.len()).rev() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                shuffled.swap(i, (state % (i as u64 + 1)) as usize);
            }
            prop_assert_eq!(ChangeClusterKey::of(&members), ChangeClusterKey::of(&shuffled));
        }
    }
}
