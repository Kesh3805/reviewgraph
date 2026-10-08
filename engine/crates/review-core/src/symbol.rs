//! Symbol primitives shared by the analyzers, the identity rules and the graph: symbol kinds,
//! module paths and 128-bit hashes.

use std::fmt;
use std::str::FromStr;

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::location::RepoPath;

/// What kind of declaration a symbol is. The id string is part of `SymbolId` (ADR-005).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum SymbolKind {
    Module,
    Namespace,
    Class,
    Interface,
    Enum,
    EnumMember,
    TypeAlias,
    Function,
    Method,
    Getter,
    Setter,
    Constructor,
    Property,
    Field,
    Variable,
    Constant,
    Parameter,
}

impl SymbolKind {
    pub const ALL: [SymbolKind; 17] = [
        Self::Module,
        Self::Namespace,
        Self::Class,
        Self::Interface,
        Self::Enum,
        Self::EnumMember,
        Self::TypeAlias,
        Self::Function,
        Self::Method,
        Self::Getter,
        Self::Setter,
        Self::Constructor,
        Self::Property,
        Self::Field,
        Self::Variable,
        Self::Constant,
        Self::Parameter,
    ];

    /// The stable string used in `SymbolId` (`.../method`).
    pub const fn as_id_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Namespace => "namespace",
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::EnumMember => "enum_member",
            Self::TypeAlias => "type_alias",
            Self::Function => "function",
            Self::Method => "method",
            Self::Getter => "get",
            Self::Setter => "set",
            Self::Constructor => "constructor",
            Self::Property => "property",
            Self::Field => "field",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Parameter => "parameter",
        }
    }

    pub fn from_id_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_id_str() == s)
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_id_str())
    }
}

/// A file path without its final extension (ADR-005's `module_path`): `src/a.service.ts` gives
/// `src/a.service`. Computed from a [`RepoPath`], never from free text.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ModulePath(String);

impl ModulePath {
    pub fn of(path: &RepoPath) -> Self {
        Self(path.module_path().to_owned())
    }

    /// Wraps a module path that [`crate::symbol_id::SymbolIdParts::parse`] or
    /// [`crate::symbol_id::module_path_for`] has already validated. Everything else must go
    /// through one of those, so an unchecked module path cannot enter an id.
    pub(crate) fn from_checked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModulePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ModulePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ModulePath({})", self.0)
    }
}

/// 128-bit content hash (the first 16 bytes of a blake3 digest). Wire form: 32 lowercase hex
/// characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash128([u8; 16]);

impl Hash128 {
    /// The unset value analyzers use before TSA-007 fills hashes in.
    pub const ZERO: Self = Self([0u8; 16]);

    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// `blake3(domain || bytes)[..16]`. The domain separates hash families.
    pub fn of(domain: &str, bytes: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain.as_bytes());
        hasher.update(&[0]);
        hasher.update(bytes);
        let mut out = [0u8; 16];
        out.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        Self(out)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 16]
    }
}

/// Size of the bottom-k shingle sketch (TSA-007). Exact while a body has at most this many
/// distinct shingles.
pub const SHINGLE_K: usize = 256;

/// Normalized body shingles (TSA-007): a sorted, de-duplicated bottom-k sample of token n-gram
/// hashes. The matcher compares two bodies through [`ShingleSet::jaccard`].
///
/// The type lives here rather than in `analysis-ir` because the rename/move matcher
/// (`review-core::matcher`) needs it and may not depend on the IR.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct ShingleSet(pub Vec<u32>);

impl ShingleSet {
    /// Builds a sketch from hashed n-grams: sorted, de-duplicated, capped at [`SHINGLE_K`].
    pub fn of(hashes: impl IntoIterator<Item = u32>) -> Self {
        let mut all: Vec<u32> = hashes.into_iter().collect();
        all.sort_unstable();
        all.dedup();
        all.truncate(SHINGLE_K);
        Self(all)
    }

    /// Number of distinct shingles kept.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no shingle was kept.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Bottom-k Jaccard estimate of two bodies: merge both sorted sets, take the `SHINGLE_K`
    /// smallest of the union and report the fraction that appear in both. While both sets are
    /// exact (at most `SHINGLE_K` distinct shingles) this is the exact Jaccard.
    pub fn jaccard(&self, other: &Self) -> f32 {
        let (mut left, mut right) = (0usize, 0usize);
        let (mut shared, mut taken) = (0usize, 0usize);
        while taken < SHINGLE_K {
            let next = match (self.0.get(left), other.0.get(right)) {
                (Some(&a), Some(&b)) => a.min(b),
                (Some(&a), None) => a,
                (None, Some(&b)) => b,
                (None, None) => break,
            };
            let in_left = self.0.get(left) == Some(&next);
            let in_right = other.0.get(right) == Some(&next);
            if in_left {
                left += 1;
            }
            if in_right {
                right += 1;
            }
            if in_left && in_right {
                shared += 1;
            }
            taken += 1;
        }
        match taken {
            0 => f32::from(u8::from(self.0.is_empty() && other.0.is_empty())),
            n => shared as f32 / n as f32,
        }
    }
}

impl fmt::Display for Hash128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Hash128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash128({self})")
    }
}

impl FromStr for Hash128 {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &str| CoreError::InvalidId {
            kind: "Hash128",
            reason: reason.to_owned(),
        };
        if s.len() != 32 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(invalid("expected 32 lowercase hex characters"));
        }
        let mut out = [0u8; 16];
        hex::decode_to_slice(s, &mut out).map_err(|e| invalid(&e.to_string()))?;
        Ok(Self(out))
    }
}

impl Serialize for Hash128 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hash128 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Hash128 {
    fn schema_name() -> String {
        "Hash128".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("Hash128", "^[0-9a-f]{32}$")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_id_strings_are_stable() {
        let listed: Vec<&str> = SymbolKind::ALL.iter().map(|k| k.as_id_str()).collect();
        assert_eq!(
            listed,
            [
                "module",
                "namespace",
                "class",
                "interface",
                "enum",
                "enum_member",
                "type_alias",
                "function",
                "method",
                "get",
                "set",
                "constructor",
                "property",
                "field",
                "variable",
                "constant",
                "parameter"
            ]
        );
        for kind in SymbolKind::ALL {
            assert_eq!(SymbolKind::from_id_str(kind.as_id_str()), Some(kind));
        }
        assert_eq!(SymbolKind::from_id_str("nope"), None);
    }

    #[test]
    fn module_path_drops_the_final_extension() {
        let p = RepoPath::new("src/a.service.ts").unwrap();
        assert_eq!(ModulePath::of(&p).as_str(), "src/a.service");
    }

    #[test]
    fn hash128_round_trips_and_separates_domains() {
        let a = Hash128::of("rg.body.v1", b"x");
        let b = Hash128::of("rg.sig.v1", b"x");
        assert_ne!(a, b);
        assert_eq!(a.to_string().parse::<Hash128>().unwrap(), a);
        assert_eq!(a.to_string().len(), 32);
        assert!(Hash128::ZERO.is_zero());
        assert!("zz".parse::<Hash128>().is_err());
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Hash128>(&json).unwrap(), a);
    }

    #[test]
    fn shingle_set_is_sorted_deduplicated_and_capped() {
        let set = ShingleSet::of([5, 1, 5, 3, 2]);
        assert_eq!(set.0, vec![1, 2, 3, 5]);
        assert_eq!(set.len(), 4);
        assert!(!set.is_empty());
        let big = ShingleSet::of((0..SHINGLE_K + 50).map(|i| i as u32));
        assert_eq!(big.len(), SHINGLE_K);
        assert_eq!(big.0[0], 0);
        assert_eq!(big.0[SHINGLE_K - 1], (SHINGLE_K - 1) as u32);
    }

    #[test]
    fn shingle_jaccard_bounds() {
        let a = ShingleSet::of([1, 2, 3, 4]);
        assert_eq!(a.jaccard(&a), 1.0);
        assert_eq!(ShingleSet::default().jaccard(&ShingleSet::default()), 1.0);
        assert_eq!(
            ShingleSet::default().jaccard(&ShingleSet::of([1, 2])),
            0.0,
            "one empty set can share nothing"
        );
        // One differing shingle out of five: 4/5.
        let b = ShingleSet::of([1, 2, 3, 4, 9]);
        assert!((a.jaccard(&b) - 0.8).abs() < 1e-6);
        assert!((a.jaccard(&ShingleSet::of([7, 8])) - 0.0).abs() < 1e-6);
    }
}
