//! Location primitives (DOM-004): validated repository paths, content hashes and source ranges.
//!
//! `RepoPath` is the system's path-traversal guard. Every filesystem join in later crates takes
//! a `RepoPath`, never a `&str`, and serde validates it too, so JSON cannot bypass the guard.

use std::fmt;
use std::str::FromStr;

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

const MAX_PATH_BYTES: usize = 4096;

/// A normalized, repository-relative path using forward slashes.
///
/// Rejected: empty, leading `/`, drive prefix (`C:`), `\`, NUL, `.` or `..` segments, empty
/// segments (`//`, trailing `/`) and anything over 4096 bytes. The path is stored exactly as
/// given; there is no case folding because the repository may be case-sensitive.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct RepoPath(String);

impl RepoPath {
    pub fn new(s: impl Into<String>) -> Result<Self, CoreError> {
        let s = s.into();
        let reject = |reason: &'static str| Err(CoreError::invalid_repo_path(&s, reason));
        if s.is_empty() {
            return reject("path is empty");
        }
        if s.len() > MAX_PATH_BYTES {
            return reject("path is longer than 4096 bytes");
        }
        if s.starts_with('/') {
            return reject("path must be relative");
        }
        let bytes = s.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return reject("drive prefixes are not allowed");
        }
        if s.contains('\\') {
            return reject("backslashes are not allowed");
        }
        if s.contains('\0') {
            return reject("NUL bytes are not allowed");
        }
        for segment in s.split('/') {
            match segment {
                "" => return reject("empty path segment"),
                "." | ".." => return reject("`.` and `..` segments are not allowed"),
                _ => {}
            }
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// Byte index of the final extension dot within the whole path, if the file name has one.
    /// Dotfiles (`.eslintrc`) and trailing dots have no extension.
    fn extension_dot(&self) -> Option<usize> {
        let name = self.file_name();
        let dot = name.rfind('.')?;
        if dot == 0 || dot + 1 == name.len() {
            return None;
        }
        Some(self.0.len() - name.len() + dot)
    }

    /// The path without its final extension (ADR-005's `module_path`):
    /// `src/a.service.ts` gives `src/a.service`.
    pub fn module_path(&self) -> &str {
        match self.extension_dot() {
            Some(dot) => &self.0[..dot],
            None => &self.0,
        }
    }

    /// The final extension without the dot.
    pub fn extension(&self) -> Option<&str> {
        self.extension_dot().map(|dot| &self.0[dot + 1..])
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepoPath({:?})", self.0)
    }
}

impl FromStr for RepoPath {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl<'de> Deserialize<'de> for RepoPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

/// Blake3 hash over raw file bytes. Wire form: 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl FromStr for ContentHash {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: String| CoreError::InvalidId {
            kind: "ContentHash",
            reason,
        };
        if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(invalid("expected 64 lowercase hex characters".to_owned()));
        }
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out).map_err(|e| invalid(e.to_string()))?;
        Ok(Self(out))
    }
}

impl Serialize for ContentHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ContentHash {
    fn schema_name() -> String {
        "ContentHash".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("ContentHash", "^[0-9a-f]{64}$")
    }
}

/// A point in a source file.
///
/// `line` is **1-based**. `column` is a **0-based UTF-8 byte offset** within the line, matching
/// tree-sitter columns. Ordering is by line, then column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

impl Position {
    pub fn new(line: u32, column: u32) -> Result<Self, CoreError> {
        if line == 0 {
            return Err(CoreError::OutOfRange {
                field: "position.line",
                value: "0 (lines are 1-based)".to_owned(),
            });
        }
        Ok(Self { line, column })
    }
}

impl<'de> Deserialize<'de> for Position {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            line: u32,
            column: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.line, raw.column).map_err(serde::de::Error::custom)
    }
}

/// A span between two [`Position`]s. Invariant: `start <= end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRange {
    pub start: Position,
    pub end: Position,
}

impl SourceRange {
    pub fn new(start: Position, end: Position) -> Result<Self, CoreError> {
        if start > end {
            return Err(CoreError::OutOfRange {
                field: "source_range",
                value: format!(
                    "start {}:{} is after end {}:{}",
                    start.line, start.column, end.line, end.column
                ),
            });
        }
        Ok(Self { start, end })
    }
}

impl<'de> Deserialize<'de> for SourceRange {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            start: Position,
            end: Position,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.start, raw.end).map_err(serde::de::Error::custom)
    }
}

/// 1-based inclusive line range. Invariant: `1 <= start <= end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    pub fn new(start: u32, end: u32) -> Result<Self, CoreError> {
        if start == 0 || start > end {
            return Err(CoreError::OutOfRange {
                field: "line_range",
                value: format!("{start}..{end} (need 1 <= start <= end)"),
            });
        }
        Ok(Self { start, end })
    }

    pub fn contains(&self, line: u32) -> bool {
        self.start <= line && line <= self.end
    }
}

impl<'de> Deserialize<'de> for LineRange {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            start: u32,
            end: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.start, raw.end).map_err(serde::de::Error::custom)
    }
}

/// Which side of a diff a location refers to.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DiffSide {
    Base,
    Head,
}

/// A place in a file at one side of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub path: RepoPath,
    pub side: DiffSide,
    pub lines: LineRange,
    pub range: Option<SourceRange>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn repo_path_rejects_traversal_absolute_backslash_nul_empty_segments() {
        let bad = [
            "",
            "/",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a/..",
            "..",
            ".",
            "./a",
            "a/.",
            "a//b",
            "a/",
            "C:/x",
            "c:x",
            "a\\b",
            "a\\..\\b",
            "a\0b",
            "a/\0",
        ];
        for p in bad {
            assert!(RepoPath::new(p).is_err(), "{p:?} should be rejected");
        }
        assert!(RepoPath::new("a".repeat(4097)).is_err());
        assert!(RepoPath::new("a".repeat(4096)).is_ok());
    }

    #[test]
    fn repo_path_accepts_ordinary_paths() {
        for p in [
            "a",
            "src/a.ts",
            ".github/workflows/ci.yml",
            "a..b/c",
            "..x/y",
            "dir/.hidden",
            "ünï/cødé.ts",
            "a b/c",
        ] {
            let rp = RepoPath::new(p).unwrap();
            assert_eq!(rp.as_str(), p);
            assert_eq!(rp.to_string(), p);
        }
    }

    #[test]
    fn repo_path_deserialize_validates() {
        assert_eq!(
            serde_json::from_str::<RepoPath>("\"src/a.ts\"")
                .unwrap()
                .as_str(),
            "src/a.ts"
        );
        for bad in ["\"../x\"", "\"/abs\"", "\"\"", "\"a\\\\b\"", "\"a//b\""] {
            assert!(serde_json::from_str::<RepoPath>(bad).is_err(), "{bad}");
        }
        let loc = r#"{"path":"../x","side":"head","lines":{"start":1,"end":1},"range":null}"#;
        assert!(serde_json::from_str::<SourceLocation>(loc).is_err());
        assert_eq!(
            serde_json::to_string(&RepoPath::new("a/b").unwrap()).unwrap(),
            "\"a/b\""
        );
    }

    #[test]
    fn module_path_strips_final_extension_only() {
        let cases = [
            ("src/a.service.ts", "src/a.service", Some("ts")),
            ("src/a.ts", "src/a", Some("ts")),
            ("README", "README", None),
            (".eslintrc", ".eslintrc", None),
            ("dir.v2/file", "dir.v2/file", None),
            ("dir.v2/file.d.ts", "dir.v2/file.d", Some("ts")),
            ("src/trailing.", "src/trailing.", None),
        ];
        for (path, module, ext) in cases {
            let p = RepoPath::new(path).unwrap();
            assert_eq!(p.module_path(), module, "{path}");
            assert_eq!(p.extension(), ext, "{path}");
        }
    }

    #[test]
    fn content_hash_golden() {
        let h = ContentHash::of(b"hello");
        assert_eq!(
            h.to_string(),
            "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f"
        );
        assert_eq!(h.to_string().parse::<ContentHash>().unwrap(), h);
        assert!(h.to_string().to_uppercase().parse::<ContentHash>().is_err());
        assert!(serde_json::from_str::<ContentHash>("\"abc\"").is_err());
    }

    #[test]
    fn line_range_rejects_zero_and_inverted() {
        assert!(LineRange::new(0, 3).is_err());
        assert!(LineRange::new(5, 4).is_err());
        assert!(LineRange::new(1, 1).is_ok());
        assert!(serde_json::from_str::<LineRange>(r#"{"start":0,"end":1}"#).is_err());
        assert!(serde_json::from_str::<LineRange>(r#"{"start":3,"end":2}"#).is_err());
        let r = LineRange::new(2, 4).unwrap();
        assert!(r.contains(2) && r.contains(4) && !r.contains(5) && !r.contains(1));
    }

    #[test]
    fn source_range_rejects_inverted() {
        let a = Position::new(3, 4).unwrap();
        let b = Position::new(3, 2).unwrap();
        let c = Position::new(2, 99).unwrap();
        assert!(SourceRange::new(a, b).is_err());
        assert!(SourceRange::new(a, c).is_err());
        assert!(SourceRange::new(b, a).is_ok());
        assert!(SourceRange::new(a, a).is_ok());
        assert!(Position::new(0, 0).is_err());
        let json = r#"{"start":{"line":3,"column":4},"end":{"line":3,"column":2}}"#;
        assert!(serde_json::from_str::<SourceRange>(json).is_err());
        assert!(serde_json::from_str::<Position>(r#"{"line":0,"column":0}"#).is_err());
    }

    #[test]
    fn diff_side_wire_names() {
        assert_eq!(serde_json::to_string(&DiffSide::Base).unwrap(), "\"base\"");
        assert_eq!(serde_json::to_string(&DiffSide::Head).unwrap(), "\"head\"");
    }

    fn segment() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_.-]{1,12}".prop_filter("not . or ..", |s| s != "." && s != "..")
    }

    proptest! {
        #[test]
        fn repo_path_valid_inputs_roundtrip(segments in prop::collection::vec(segment(), 1..6)) {
            let raw = segments.join("/");
            let path = RepoPath::new(raw.clone()).unwrap();
            prop_assert_eq!(path.as_str(), raw.as_str());
            let json = serde_json::to_string(&path).unwrap();
            prop_assert_eq!(serde_json::from_str::<RepoPath>(&json).unwrap(), path.clone());
            prop_assert_eq!(path.to_string().parse::<RepoPath>().unwrap(), path);
        }
    }
}
