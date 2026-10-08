//! The typed edge model (CG-002): [`EdgeKind`] lives in [`crate::edge_kind`]; this module owns
//! the payload every producer (linker, framework mapper, type checker) and every consumer
//! (queries, storage, verification) shares — confidence, how it was resolved, why it was
//! produced, its flags and where it was observed.
//!
//! Uncertainty is recorded on every edge (PRD §19, invariant 7). Reverse relations
//! (`CALLED_BY`, `DEPENDED_ON_BY`, `TESTED_BY`) are never part of an [`Edge`]; they are views
//! over the reverse index, answered by [`crate::query`].
//!
//! **Invariant.** When [`Edge::location`] is `Some`, its `file` equals [`Edge::origin_file`]:
//! the ownership column used for per-file re-linking (INC-005/006) and the file the location is
//! reported in are the same file. Every constructor in this module keeps the two in sync, and
//! the codec re-derives one from the other, so the round-trip is lossless.

use std::fmt;

use review_core::location::RepoPath;
use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::edge_kind::EdgeKind;
use crate::node_id::NodeKey;
use crate::schema_util::string_enum_schema;

pub use crate::confidence::Confidence;

/// Where an edge was observed: the file that produced it plus a 1-based position.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct Location {
    /// Repository-relative path of the file the edge was observed in.
    pub file: RepoPath,
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub col: u32,
}

impl Location {
    pub fn new(file: RepoPath, line: u32, col: u32) -> Self {
        Self { file, line, col }
    }
}

/// How a reference was resolved to an edge. The confidence table is keyed by it
/// (see [`crate::confidence`]).
///
/// `repr(u8)` discriminants are persisted in `resolved_by_kinds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ResolvedBy {
    Structural = 0,
    Import = 1,
    ThisMember = 2,
    DiConstructor = 3,
    TypeAnnotation = 4,
    NameUnique = 5,
    NameAmbiguous = 6,
    Framework = 7,
    TypeChecker = 8,
    Heuristic = 9,
}

impl ResolvedBy {
    pub const ALL: [ResolvedBy; 10] = [
        Self::Structural,
        Self::Import,
        Self::ThisMember,
        Self::DiConstructor,
        Self::TypeAnnotation,
        Self::NameUnique,
        Self::NameAmbiguous,
        Self::Framework,
        Self::TypeChecker,
        Self::Heuristic,
    ];

    /// The PRD wire spelling stored in `resolved_by_kinds`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Structural => "STRUCTURAL",
            Self::Import => "IMPORT",
            Self::ThisMember => "THIS_MEMBER",
            Self::DiConstructor => "DI_CONSTRUCTOR",
            Self::TypeAnnotation => "TYPE_ANNOTATION",
            Self::NameUnique => "NAME_UNIQUE",
            Self::NameAmbiguous => "NAME_AMBIGUOUS",
            Self::Framework => "FRAMEWORK",
            Self::TypeChecker => "TYPE_CHECKER",
            Self::Heuristic => "HEURISTIC",
        }
    }

    pub fn from_str_exact(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(raw: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_u8() == raw)
    }
}

impl fmt::Display for ResolvedBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ResolvedBy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s).ok_or_else(|| format!("ResolvedBy has no member {s:?}"))
    }
}

impl Serialize for ResolvedBy {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ResolvedBy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str_exact(&raw)
            .ok_or_else(|| D::Error::custom(format!("unknown ResolvedBy {raw:?}")))
    }
}

impl JsonSchema for ResolvedBy {
    fn schema_name() -> String {
        "ResolvedBy".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let values = ResolvedBy::ALL.map(|v| v.as_str());
        string_enum_schema("ResolvedBy", &values)
    }
}

/// Which stage produced an edge row. `repr(u8)` discriminants are persisted in
/// `provenance_kinds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Provenance {
    Analyzer = 0,
    Framework = 1,
    Linker = 2,
    TypeChecker = 3,
    Heuristic = 4,
    Policy = 5,
}

impl Provenance {
    pub const ALL: [Provenance; 6] = [
        Self::Analyzer,
        Self::Framework,
        Self::Linker,
        Self::TypeChecker,
        Self::Heuristic,
        Self::Policy,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Analyzer => "ANALYZER",
            Self::Framework => "FRAMEWORK",
            Self::Linker => "LINKER",
            Self::TypeChecker => "TYPE_CHECKER",
            Self::Heuristic => "HEURISTIC",
            Self::Policy => "POLICY",
        }
    }

    pub fn from_str_exact(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(raw: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_u8() == raw)
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Provenance {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s).ok_or_else(|| format!("Provenance has no member {s:?}"))
    }
}

impl Serialize for Provenance {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Provenance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str_exact(&raw)
            .ok_or_else(|| D::Error::custom(format!("unknown Provenance {raw:?}")))
    }
}

impl JsonSchema for Provenance {
    fn schema_name() -> String {
        "Provenance".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let values = Provenance::ALL.map(|v| v.as_str());
        string_enum_schema("Provenance", &values)
    }
}

/// Bit flags carried by an edge (`graph_edges.flags`, a `smallint`).
///
/// A hand-rolled bitset rather than the `bitflags!` macro so the persisted values stay
/// explicit and comparable with `graph-storage`'s `EdgeFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct EdgeFlags(u8);

impl EdgeFlags {
    /// The target is constructed by the call (`new X()`).
    pub const INSTANTIATES: Self = Self(1);
    /// The reference came from a decorator or annotation.
    pub const DECORATOR: Self = Self(2);
    /// Type-only reference: erased at runtime.
    pub const TYPE_ONLY: Self = Self(4);
    /// Resolution was dynamic (a fan-out candidate or an unresolvable receiver).
    pub const DYNAMIC: Self = Self(8);
    /// The edge maps an entity onto a table.
    pub const MAPS_TABLE: Self = Self(16);
    /// The guard applies repository-wide rather than to one endpoint.
    pub const GLOBAL_SCOPE: Self = Self(32);

    pub const EMPTY: Self = Self(0);
    pub const ALL_BITS: Self = Self(63);

    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every flag name present, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        const NAMED: [(EdgeFlags, &str); 6] = [
            (EdgeFlags::INSTANTIATES, "INSTANTIATES"),
            (EdgeFlags::DECORATOR, "DECORATOR"),
            (EdgeFlags::TYPE_ONLY, "TYPE_ONLY"),
            (EdgeFlags::DYNAMIC, "DYNAMIC"),
            (EdgeFlags::MAPS_TABLE, "MAPS_TABLE"),
            (EdgeFlags::GLOBAL_SCOPE, "GLOBAL_SCOPE"),
        ];
        NAMED
            .into_iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| name)
            .collect()
    }
}

impl std::ops::BitOr for EdgeFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for EdgeFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl fmt::Display for EdgeFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.names().join("|"))
    }
}

impl Serialize for EdgeFlags {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.0)
    }
}

impl<'de> Deserialize<'de> for EdgeFlags {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(u8::deserialize(deserializer)?))
    }
}

impl JsonSchema for EdgeFlags {
    fn schema_name() -> String {
        "EdgeFlags".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut object = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::InstanceType::Number.into()),
            ..schemars::schema::SchemaObject::default()
        };
        object.metadata = Some(Box::new(schemars::schema::Metadata {
            description: Some(
                "Bit set over INSTANTIATES|DECORATOR|TYPE_ONLY|DYNAMIC|MAPS_TABLE|GLOBAL_SCOPE."
                    .to_owned(),
            ),
            ..schemars::schema::Metadata::default()
        }));
        Schema::Object(object)
    }
}

/// `(source, kind, target)` — the logical identity of an edge (clarification C5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeIdentity {
    pub source: NodeKey,
    pub kind: EdgeKind,
    pub target: NodeKey,
}

impl fmt::Display for EdgeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -[{}]-> {}", self.source, self.kind, self.target)
    }
}

/// One resolved edge, as produced by the linker and the framework mapper.
///
/// `Ord` orders by identity first (so it matches [`Edge::identity`]) and then by the payload,
/// which makes the order total and the sort of an edge list reproducible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub source: NodeKey,
    pub target: NodeKey,
    pub confidence: Confidence,
    pub resolved_by: ResolvedBy,
    pub provenance: Provenance,
    pub flags: EdgeFlags,
    /// Where the edge was observed. Its `file` always equals [`Edge::origin_file`].
    pub location: Option<Location>,
    /// How many occurrences were merged into this edge (saturating).
    pub occurrences: u32,
    /// File whose content produced the edge; the re-link unit for incremental updates.
    pub origin_file: Option<RepoPath>,
}

impl Edge {
    /// A single-occurrence edge with no location and no flags.
    pub fn new(
        kind: EdgeKind,
        source: NodeKey,
        target: NodeKey,
        confidence: Confidence,
        resolved_by: ResolvedBy,
        provenance: Provenance,
    ) -> Self {
        Self {
            kind,
            source,
            target,
            confidence,
            resolved_by,
            provenance,
            flags: EdgeFlags::EMPTY,
            location: None,
            occurrences: 1,
            origin_file: None,
        }
    }

    /// Records where the edge was observed; the location's file becomes the origin file.
    pub fn with_location(mut self, location: Location) -> Self {
        self.origin_file = Some(location.file.clone());
        self.location = Some(location);
        self
    }

    /// Sets the file whose content produced the edge. When a location is already recorded it
    /// wins, keeping the [`Self::location`] / [`Self::origin_file`] invariant intact.
    pub fn with_origin_file(mut self, file: RepoPath) -> Self {
        if self.location.is_none() {
            self.origin_file = Some(file);
        }
        self
    }

    pub fn with_flags(mut self, flags: EdgeFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn with_occurrences(mut self, occurrences: u32) -> Self {
        self.occurrences = occurrences;
        self
    }

    /// `(source, kind, target)`.
    pub const fn identity(&self) -> EdgeIdentity {
        EdgeIdentity {
            source: self.source,
            kind: self.kind,
            target: self.target,
        }
    }

    /// Merges another edge with the same identity into `self`.
    ///
    /// Semantics (property-tested for commutativity and associativity):
    /// * `confidence` / `resolved_by` / `provenance` come from the higher confidence, ties
    ///   broken by the lower [`ResolvedBy`] discriminant so the winner never depends on order;
    /// * `flags` are unioned;
    /// * `occurrences` are summed with saturation;
    /// * `location` and `origin_file` keep the lexicographically smallest value.
    ///
    /// The caller guarantees both edges share [`Edge::identity`]; the builder is what performs
    /// the grouping.
    pub fn merge_occurrence(&mut self, other: &Edge) {
        let other_wins = (other.confidence, Reverse(other.resolved_by))
            > (self.confidence, Reverse(self.resolved_by));
        if other_wins {
            self.confidence = other.confidence;
            self.resolved_by = other.resolved_by;
            self.provenance = other.provenance;
        }
        self.flags = self.flags.union(other.flags);
        self.occurrences = self.occurrences.saturating_add(other.occurrences);
        self.location = match (&self.location, &other.location) {
            (Some(a), Some(b)) => Some(if b < a { b.clone() } else { a.clone() }),
            (None, b) => b.clone(),
            (a, None) => a.clone(),
        };
        self.origin_file = match (&self.origin_file, &other.origin_file) {
            (Some(a), Some(b)) => Some(if b < a { b.clone() } else { a.clone() }),
            (None, b) => b.clone(),
            (a, None) => a.clone(),
        };
        if let Some(location) = &self.location {
            self.origin_file = Some(location.file.clone());
        }
    }

    /// Estimated heap footprint, used by the byte-weighted graph cache (GS-008).
    pub fn heap_size_bytes(&self) -> usize {
        64 + self.origin_file.as_ref().map_or(0, |f| f.as_str().len())
    }
}

/// Reverses an ordering so that "smaller discriminant wins" can be expressed with `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reverse<T>(T);

impl<T: Ord> Ord for Reverse<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.cmp(&self.0)
    }
}

impl<T: Ord> PartialOrd for Reverse<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::cmp::Ord for Edge {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity()
            .cmp(&other.identity())
            .then_with(|| self.confidence.cmp(&other.confidence))
            .then_with(|| Reverse(self.resolved_by).cmp(&Reverse(other.resolved_by)))
            .then_with(|| self.provenance.cmp(&other.provenance))
            .then_with(|| self.flags.cmp(&other.flags))
            .then_with(|| self.location.cmp(&other.location))
            .then_with(|| self.origin_file.cmp(&other.origin_file))
            .then_with(|| self.occurrences.cmp(&other.occurrences))
    }
}

impl PartialOrd for Edge {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::edge_kind::{Direction, EdgeSelector, ReverseView};
    use crate::node_id::NodeId;
    use proptest::prelude::*;

    fn node_key(s: &str) -> NodeKey {
        NodeId::from_canonical(s.to_owned()).key()
    }

    fn file(s: &str) -> RepoPath {
        RepoPath::new(s).unwrap()
    }

    fn edge(kind: EdgeKind, source: &str, target: &str, permille: u16) -> Edge {
        Edge::new(
            kind,
            node_key(source),
            node_key(target),
            Confidence::from_f32(f32::from(permille) / 1000.0),
            ResolvedBy::NameUnique,
            Provenance::Linker,
        )
    }

    #[test]
    fn resolved_by_and_provenance_roundtrip_str_and_serde() {
        for v in ResolvedBy::ALL {
            assert_eq!(ResolvedBy::from_str_exact(v.as_str()), Some(v));
            assert_eq!(ResolvedBy::from_u8(v.as_u8()), Some(v));
            assert_eq!(v.to_string(), v.as_str());
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, format!("\"{}\"", v.as_str()));
            assert_eq!(serde_json::from_str::<ResolvedBy>(&json).unwrap(), v);
        }
        assert_eq!(ResolvedBy::ALL.len(), 10);
        assert_eq!(ResolvedBy::Structural.as_str(), "STRUCTURAL");
        assert!(ResolvedBy::from_str_exact("CALLED_BY").is_none());

        for v in Provenance::ALL {
            assert_eq!(Provenance::from_str_exact(v.as_str()), Some(v));
            assert_eq!(Provenance::from_u8(v.as_u8()), Some(v));
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(serde_json::from_str::<Provenance>(&json).unwrap(), v);
        }
        assert_eq!(Provenance::ALL.len(), 6);
        assert_eq!(Provenance::Linker.as_str(), "LINKER");
        assert!("nope".parse::<ResolvedBy>().is_err());
        assert!("nope".parse::<Provenance>().is_err());
    }

    #[test]
    fn edge_flags_bits_match_storage_values() {
        assert_eq!(EdgeFlags::INSTANTIATES.bits(), 1);
        assert_eq!(EdgeFlags::DECORATOR.bits(), 2);
        assert_eq!(EdgeFlags::TYPE_ONLY.bits(), 4);
        assert_eq!(EdgeFlags::DYNAMIC.bits(), 8);
        assert_eq!(EdgeFlags::MAPS_TABLE.bits(), 16);
        assert_eq!(EdgeFlags::GLOBAL_SCOPE.bits(), 32);
        assert!(EdgeFlags::EMPTY.is_empty());
        let both = EdgeFlags::INSTANTIATES | EdgeFlags::DECORATOR;
        assert!(both.contains(EdgeFlags::INSTANTIATES));
        assert!(both.contains(EdgeFlags::DECORATOR));
        assert!(!both.contains(EdgeFlags::DYNAMIC));
        assert_eq!(both.bits(), 3);
        assert_eq!(both.to_string(), "INSTANTIATES|DECORATOR");
        assert_eq!(
            EdgeFlags::from_bits(20).names(),
            vec!["TYPE_ONLY", "MAPS_TABLE"]
        );
        let json = serde_json::to_string(&EdgeFlags::DYNAMIC).unwrap();
        assert_eq!(json, "8");
        assert_eq!(
            serde_json::from_str::<EdgeFlags>(&json).unwrap(),
            EdgeFlags::DYNAMIC
        );
        let schema = serde_json::to_value(schemars::schema_for!(EdgeFlags)).unwrap();
        assert_eq!(schema["type"], "number");
    }

    #[test]
    fn edge_ord_is_total_and_matches_identity_order() {
        let a = edge(EdgeKind::Calls, "a", "b", 900);
        let mut b = edge(EdgeKind::Calls, "a", "b", 500);
        assert_eq!(
            a.identity(),
            EdgeIdentity {
                source: node_key("a"),
                kind: EdgeKind::Calls,
                target: node_key("b"),
            }
        );
        assert_eq!(a.identity(), b.identity());
        // Same identity, different payload: ordered by payload, not equal.
        assert_ne!(a, b);
        assert_eq!(a.cmp(&b), std::cmp::Ordering::Greater);

        let other = edge(EdgeKind::Imports, "a", "b", 500);
        assert_eq!(
            a.cmp(&other),
            a.identity().cmp(&other.identity()),
            "when identities differ, edge order equals identity order"
        );

        let mut c = a.clone();
        c.confidence = b.confidence;
        c.resolved_by = b.resolved_by;
        c.provenance = b.provenance;
        c.occurrences = b.occurrences;
        assert_eq!(a.cmp(&c), std::cmp::Ordering::Greater);

        let mut list = vec![other.clone(), b.clone(), a.clone()];
        list.sort();
        b.occurrences = 1;
        assert_eq!(list, vec![other, b, a]);
    }

    #[test]
    fn merge_occurrence_keeps_min_location_max_confidence_and_sums() {
        let first = Edge::new(
            EdgeKind::Calls,
            node_key("a"),
            node_key("b"),
            Confidence::from_f32(0.6),
            ResolvedBy::NameUnique,
            Provenance::Linker,
        )
        .with_location(Location::new(file("src/b.ts"), 40, 1));
        let second = Edge::new(
            EdgeKind::Calls,
            node_key("a"),
            node_key("b"),
            Confidence::from_f32(0.95),
            ResolvedBy::Import,
            Provenance::Linker,
        )
        .with_location(Location::new(file("src/b.ts"), 12, 3))
        .with_flags(EdgeFlags::DYNAMIC);

        let mut merged = first.clone();
        merged.merge_occurrence(&second);
        assert_eq!(merged.confidence.as_permille(), 950);
        assert_eq!(merged.resolved_by, ResolvedBy::Import);
        assert_eq!(merged.location.as_ref().unwrap().line, 12);
        assert_eq!(merged.occurrences, 2);
        assert!(merged.flags.contains(EdgeFlags::DYNAMIC));
        assert_eq!(
            merged.origin_file.as_ref().unwrap().as_str(),
            "src/b.ts",
            "location file stays authoritative"
        );

        let mut other_way = second.clone();
        other_way.merge_occurrence(&first);
        assert_eq!(merged, other_way, "merge is commutative");

        let third = Edge::new(
            EdgeKind::Calls,
            node_key("a"),
            node_key("b"),
            Confidence::from_f32(0.3),
            ResolvedBy::NameAmbiguous,
            Provenance::Linker,
        )
        .with_location(Location::new(file("src/c.ts"), 1, 1));

        let mut left = merged.clone();
        left.merge_occurrence(&third);
        let mut right = second.clone();
        right.merge_occurrence(&third);
        let mut left_again = first.clone();
        left_again.merge_occurrence(&right);
        assert_eq!(left, left_again, "merge is associative");
        assert_eq!(left.occurrences, 3);
        assert_eq!(
            left.location.as_ref().unwrap().file.as_str(),
            "src/b.ts",
            "location ordering is (file, line, col), so src/b.ts beats src/c.ts"
        );
        assert_eq!(left.location.as_ref().unwrap().line, 12);
    }

    #[test]
    fn edge_occurrences_saturate() {
        let mut a = edge(EdgeKind::Calls, "a", "b", 900).with_occurrences(u32::MAX);
        let b = edge(EdgeKind::Calls, "a", "b", 900);
        a.merge_occurrence(&b);
        assert_eq!(a.occurrences, u32::MAX);
    }

    #[test]
    fn reverse_views_translate_to_underlying_kinds() {
        assert_eq!(
            EdgeSelector::from(ReverseView::CalledBy).underlying(),
            (EdgeKind::Calls, Direction::In)
        );
        assert_eq!(
            EdgeSelector::from(ReverseView::TestedBy).underlying(),
            (EdgeKind::Tests, Direction::In)
        );
        assert_eq!(
            EdgeSelector::from(ReverseView::DependedOnBy).underlying(),
            (EdgeKind::DependsOn, Direction::In)
        );
    }

    #[test]
    fn edge_serde_roundtrip() {
        let e = edge(EdgeKind::Extends, "a", "b", 850)
            .with_location(Location::new(file("src/a.ts"), 3, 7))
            .with_flags(EdgeFlags::TYPE_ONLY | EdgeFlags::INSTANTIATES)
            .with_occurrences(4);
        let json = serde_json::to_string(&e).unwrap();
        let back: Edge = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.identity(), e.identity());
        assert_eq!(
            serde_json::to_value(&e).unwrap()["kind"],
            serde_json::json!("EXTENDS")
        );
        assert_eq!(
            serde_json::to_value(&e).unwrap()["resolved_by"],
            serde_json::json!("NAME_UNIQUE")
        );
        assert_eq!(
            serde_json::to_value(&e).unwrap()["provenance"],
            serde_json::json!("LINKER")
        );
    }

    #[test]
    fn edge_heap_size_accounts_for_origin_path() {
        let bare = edge(EdgeKind::Calls, "a", "b", 900);
        let with_path = bare
            .clone()
            .with_origin_file(file("src/a/very/long/path.ts"));
        assert!(with_path.heap_size_bytes() > bare.heap_size_bytes());
        assert!(bare.heap_size_bytes() >= 64);
    }

    proptest! {
        #[test]
        fn merge_occurrence_commutative_associative(
            c1 in 0u16..=1000,
            c2 in 0u16..=1000,
            c3 in 0u16..=1000,
            l1 in 1u32..500,
            l2 in 1u32..500,
            l3 in 1u32..500,
        ) {
            let mk = |permille: u16, resolved: ResolvedBy, prov: Provenance, line: u32| {
                Edge::new(
                    EdgeKind::Calls,
                    node_key("a"),
                    node_key("b"),
                    Confidence::from_f32(f32::from(permille) / 1000.0),
                    resolved,
                    prov,
                )
                .with_location(Location::new(file("src/x.ts"), line, 1))
            };
            let a = mk(c1, ResolvedBy::NameUnique, Provenance::Linker, l1);
            let b = mk(c2, ResolvedBy::NameAmbiguous, Provenance::Framework, l2);
            let c = mk(c3, ResolvedBy::Import, Provenance::Analyzer, l3);

            let mut ab = a.clone();
            ab.merge_occurrence(&b);
            let mut ba = b.clone();
            ba.merge_occurrence(&a);
            prop_assert_eq!(ab.clone(), ba);

            let mut ab_c = ab.clone();
            ab_c.merge_occurrence(&c);
            let mut bc = b.clone();
            bc.merge_occurrence(&c);
            let mut a_bc = a.clone();
            a_bc.merge_occurrence(&bc);
            prop_assert_eq!(ab_c, a_bc.clone());

            let mut ac = a.clone();
            ac.merge_occurrence(&c);
            let mut ac_b = ac.clone();
            ac_b.merge_occurrence(&b);
            prop_assert_eq!(ac_b, a_bc.clone());

            prop_assert!(ab.occurrences >= 2);
            prop_assert!(ab.confidence.as_permille() >= c1.max(c2));
            prop_assert_eq!(
                ab.location.as_ref().unwrap().line,
                l1.min(l2)
            );
        }
    }

    proptest! {
        #[test]
        fn edge_ord_is_consistent_with_eq(a_conf in 0u16..=1000, b_conf in 0u16..=1000) {
            let a = edge(EdgeKind::Calls, "a", "b", a_conf);
            let b = edge(EdgeKind::Calls, "a", "b", b_conf);
            prop_assert_eq!(a == b, a.cmp(&b) == std::cmp::Ordering::Equal);
            prop_assert_eq!(a.cmp(&b).reverse(), b.cmp(&a));
            if a != b {
                prop_assert!(a.cmp(&b) != std::cmp::Ordering::Equal);
            }
        }
    }
}
