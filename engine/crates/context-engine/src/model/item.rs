//! `ContextItem` and its parts.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

use review_core::ids::{CommitSha, SymbolKey};
use review_core::location::{DiffSide, RepoPath};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    ChangedSymbol,
    RelatedSymbol,
    Test,
    Config,
    ApiEndpoint,
    RuleDoc,
    Convention,
    SchemaChange,
    DependencyChange,
    ClassSignature,
}

impl ItemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChangedSymbol => "changed_symbol",
            Self::RelatedSymbol => "related_symbol",
            Self::Test => "test",
            Self::Config => "config",
            Self::ApiEndpoint => "api_endpoint",
            Self::RuleDoc => "rule_doc",
            Self::Convention => "convention",
            Self::SchemaChange => "schema_change",
            Self::DependencyChange => "dependency_change",
            Self::ClassSignature => "class_signature",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    Structural,
    TestMap,
    Config,
    Api,
    RuleDoc,
    Lexical,
    Semantic,
}

/// Stable item id: `blake3(kind || node key || side)[..16]`, lowercase hex on the wire.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId([u8; 16]);

impl ItemId {
    /// `node` is `None` for items without a graph node; `title` then disambiguates.
    pub fn of(kind: ItemKind, node: Option<&SymbolKey>, side: DiffSide, title: &str) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(kind.as_str().as_bytes());
        h.update(&[0]);
        match node {
            Some(k) => {
                h.update(k.as_bytes());
            }
            None => {
                h.update(title.as_bytes());
            }
        }
        h.update(&[0]);
        h.update(match side {
            DiffSide::Base => b"base".as_slice(),
            DiffSide::Head => b"head".as_slice(),
        });
        let mut out = [0u8; 16];
        out.copy_from_slice(&h.finalize().as_bytes()[..16]);
        Self(out)
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ItemId({self})")
    }
}

impl Serialize for ItemId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for ItemId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let bytes = hex::decode(&text).map_err(serde::de::Error::custom)?;
        let arr: [u8; 16] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("item id must be 32 hex characters"))?;
        Ok(Self(arr))
    }
}

/// One hop of the impact path that led to an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathStep {
    pub relation: String,
    pub target: String,
    pub confidence: f32,
}

/// The ten ranking signals of an item, each in `[0, 1]` (weights applied by CTX-005).
pub type SignalVector = crate::ranking::Signals;

/// How much of a symbol's source an excerpt keeps (CTX-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionLevel {
    /// The whole symbol body.
    Full,
    /// Signature plus changed hunks with three lines of context and elisions.
    ChangedHunks,
    /// Signature, the first 15 and last 5 lines, and lines calling the seeds.
    BodyTrimmed,
    SignatureOnly,
    /// Title and location only.
    Reference,
}

/// An inclusive range of 1-based lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElidedLines {
    pub start: u32,
    pub end: u32,
}

/// A contiguous run of source lines. `text` is exactly lines `location.start_line..=end_line`
/// of the blob at `location.commit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub location: SourceLocation,
    pub text: String,
    /// Lines skipped between the previous segment and this one.
    pub elided_before: Option<ElidedLines>,
}

/// Rendered content of an item (CTX-007 decides the form).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum ItemContent {
    Full {
        text: String,
    },
    Signature {
        text: String,
    },
    Summary {
        text: String,
    },
    Empty,
    /// Source excerpt with per-segment locations.
    Code {
        level: CompressionLevel,
        segments: Vec<Segment>,
    },
    /// Endpoint attributes, config keys or rule text.
    Structured {
        value: serde_json::Value,
    },
}

impl ItemContent {
    /// The text a model sees. Elisions between code segments are rendered as
    /// `// … lines a–b elided …`.
    pub fn text(&self) -> Cow<'_, str> {
        match self {
            Self::Full { text } | Self::Signature { text } | Self::Summary { text } => {
                Cow::Borrowed(text.as_str())
            }
            Self::Empty => Cow::Borrowed(""),
            Self::Code { segments, .. } => {
                let mut out = String::new();
                for seg in segments {
                    if let Some(gap) = seg.elided_before {
                        out.push_str(&format!("// … lines {}–{} elided …\n", gap.start, gap.end));
                    }
                    out.push_str(&seg.text);
                    if !seg.text.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Cow::Owned(out)
            }
            Self::Structured { value } => Cow::Owned(value.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: RepoPath,
    pub start_line: u32,
    pub end_line: u32,
    pub side: DiffSide,
    pub commit: CommitSha,
}

impl SourceLocation {
    /// `path:start-end@side`, the form every model-input excerpt carries.
    pub fn render(&self) -> String {
        let side = match self.side {
            DiffSide::Base => "base",
            DiffSide::Head => "head",
        };
        format!(
            "{}:{}-{}@{}",
            self.file.as_str(),
            self.start_line,
            self.end_line,
            side
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: ItemId,
    pub kind: ItemKind,
    pub node: Option<SymbolKey>,
    pub side: DiffSide,
    /// Qualified name or path.
    pub title: String,
    pub relation: Option<String>,
    pub distance: Option<u8>,
    pub path: Vec<PathStep>,
    pub sources: BTreeSet<CandidateSource>,
    pub signals: SignalVector,
    pub score: f32,
    pub content: ItemContent,
    pub locations: Vec<SourceLocation>,
    pub tokens: u32,
    pub pinned: bool,
}

impl ContextItem {
    /// Package order: pinned first, then score descending, then id.
    pub fn package_order(a: &Self, b: &Self) -> std::cmp::Ordering {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.id.cmp(&b.id))
    }
}
