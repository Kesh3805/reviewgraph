//! Edge taxonomy (CG-002): the 33 stored [`EdgeKind`]s of PRD §18, the three non-stored
//! [`ReverseView`]s, the [`EdgeSelector`] that parses either spelling, and the [`EdgeKindSet`]
//! bitset used by every filter.
//!
//! Clarification C2: `CALLED_BY`, `DEPENDED_ON_BY` and `TESTED_BY` are views over the reverse
//! index and are never stored, so double writes and drift are impossible.

use std::fmt;
use std::str::FromStr;

use schemars::gen::SchemaGenerator;
use schemars::schema::{InstanceType, NumberValidation, Schema, SchemaObject};
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::schema_util::string_enum_schema;

/// A stored edge relation. Exactly 33 variants (clarification C2).
///
/// `repr(u8)` discriminants are part of the persisted wire format and are mirrored by the
/// `edge_kinds` lookup table in `graph-storage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum EdgeKind {
    Contains = 0,
    Declares = 1,
    Imports = 2,
    Exports = 3,
    Calls = 4,
    Reads = 5,
    Writes = 6,
    Implements = 7,
    Extends = 8,
    Overrides = 9,
    References = 10,
    UsesType = 11,
    ReturnsType = 12,
    AcceptsType = 13,
    RoutesTo = 14,
    HandledBy = 15,
    Tests = 16,
    Covers = 17,
    ProducesJob = 18,
    ConsumesJob = 19,
    ReadsConfig = 20,
    WritesConfig = 21,
    ReadsTable = 22,
    WritesTable = 23,
    DependsOn = 24,
    Throws = 25,
    Catches = 26,
    Serializes = 27,
    Deserializes = 28,
    Validates = 29,
    Authorizes = 30,
    Publishes = 31,
    Subscribes = 32,
}

/// Every stored edge kind, in discriminant order.
pub const ALL_EDGE_KINDS: [EdgeKind; 33] = [
    EdgeKind::Contains,
    EdgeKind::Declares,
    EdgeKind::Imports,
    EdgeKind::Exports,
    EdgeKind::Calls,
    EdgeKind::Reads,
    EdgeKind::Writes,
    EdgeKind::Implements,
    EdgeKind::Extends,
    EdgeKind::Overrides,
    EdgeKind::References,
    EdgeKind::UsesType,
    EdgeKind::ReturnsType,
    EdgeKind::AcceptsType,
    EdgeKind::RoutesTo,
    EdgeKind::HandledBy,
    EdgeKind::Tests,
    EdgeKind::Covers,
    EdgeKind::ProducesJob,
    EdgeKind::ConsumesJob,
    EdgeKind::ReadsConfig,
    EdgeKind::WritesConfig,
    EdgeKind::ReadsTable,
    EdgeKind::WritesTable,
    EdgeKind::DependsOn,
    EdgeKind::Throws,
    EdgeKind::Catches,
    EdgeKind::Serializes,
    EdgeKind::Deserializes,
    EdgeKind::Validates,
    EdgeKind::Authorizes,
    EdgeKind::Publishes,
    EdgeKind::Subscribes,
];

impl EdgeKind {
    /// Every stored variant, in discriminant order.
    pub const ALL: [EdgeKind; 33] = ALL_EDGE_KINDS;

    /// The PRD wire spelling persisted in `edge_kinds` and shown to the API.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "CONTAINS",
            Self::Declares => "DECLARES",
            Self::Imports => "IMPORTS",
            Self::Exports => "EXPORTS",
            Self::Calls => "CALLS",
            Self::Reads => "READS",
            Self::Writes => "WRITES",
            Self::Implements => "IMPLEMENTS",
            Self::Extends => "EXTENDS",
            Self::Overrides => "OVERRIDES",
            Self::References => "REFERENCES",
            Self::UsesType => "USES_TYPE",
            Self::ReturnsType => "RETURNS_TYPE",
            Self::AcceptsType => "ACCEPTS_TYPE",
            Self::RoutesTo => "ROUTES_TO",
            Self::HandledBy => "HANDLED_BY",
            Self::Tests => "TESTS",
            Self::Covers => "COVERS",
            Self::ProducesJob => "PRODUCES_JOB",
            Self::ConsumesJob => "CONSUMES_JOB",
            Self::ReadsConfig => "READS_CONFIG",
            Self::WritesConfig => "WRITES_CONFIG",
            Self::ReadsTable => "READS_TABLE",
            Self::WritesTable => "WRITES_TABLE",
            Self::DependsOn => "DEPENDS_ON",
            Self::Throws => "THROWS",
            Self::Catches => "CATCHES",
            Self::Serializes => "SERIALIZES",
            Self::Deserializes => "DESERIALIZES",
            Self::Validates => "VALIDATES",
            Self::Authorizes => "AUTHORIZES",
            Self::Publishes => "PUBLISHES",
            Self::Subscribes => "SUBSCRIBES",
        }
    }

    /// Parses the PRD spelling.
    pub fn from_str_exact(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == s)
    }

    /// The discriminant persisted in `edge_kinds.smallint`.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Reads a persisted discriminant back.
    pub fn from_u8(raw: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_u8() == raw)
    }
}

/// A reverse relation that is answered from the reverse index and never stored.
///
/// The API accepts these names in kind filters; [`EdgeSelector`] translates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum ReverseView {
    CalledBy = 0,
    DependedOnBy = 1,
    TestedBy = 2,
}

impl ReverseView {
    pub const ALL: [ReverseView; 3] = [Self::CalledBy, Self::DependedOnBy, Self::TestedBy];

    /// The stored kind this view reads, in the `In` direction.
    pub const fn underlying(self) -> EdgeKind {
        match self {
            Self::CalledBy => EdgeKind::Calls,
            Self::DependedOnBy => EdgeKind::DependsOn,
            Self::TestedBy => EdgeKind::Tests,
        }
    }

    /// The PRD spelling: `CALLED_BY`, `DEPENDED_ON_BY`, `TESTED_BY`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CalledBy => "CALLED_BY",
            Self::DependedOnBy => "DEPENDED_ON_BY",
            Self::TestedBy => "TESTED_BY",
        }
    }

    pub fn from_str_exact(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.as_str() == s)
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for ReverseView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReverseView {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s).ok_or_else(|| format!("ReverseView has no member {s:?}"))
    }
}

impl JsonSchema for ReverseView {
    fn schema_name() -> String {
        "ReverseView".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let values = ReverseView::ALL.map(|view| view.as_str());
        string_enum_schema("ReverseView", &values)
    }
}

/// Direction of a traversal or neighbour lookup.
///
/// `Out` follows `source -> target`, `In` the reverse, `Both` walks both adjacency lists
/// (`Out` first, then `In`, so iteration order stays deterministic).
///
/// The query API uses all three; the persisted form only ever needs `Out`/`In`
/// (`EdgeSelector::underlying` never yields `Both`), which is what `graph-storage` mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Out,
    In,
    Both,
}

impl Direction {
    pub const ALL: [Direction; 3] = [Self::Out, Self::In, Self::Both];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Out => "out",
            Self::In => "in",
            Self::Both => "both",
        }
    }

    pub fn from_str_exact(s: &str) -> Option<Self> {
        match s {
            "out" => Some(Self::Out),
            "in" => Some(Self::In),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    /// The opposite direction; `Both` is its own opposite.
    pub const fn opposite(self) -> Self {
        match self {
            Self::Out => Self::In,
            Self::In => Self::Out,
            Self::Both => Self::Both,
        }
    }

    pub const fn includes_out(self) -> bool {
        matches!(self, Self::Out | Self::Both)
    }

    pub const fn includes_in(self) -> bool {
        matches!(self, Self::In | Self::Both)
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Direction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s).ok_or_else(|| format!("Direction has no member {s:?}"))
    }
}

impl JsonSchema for Direction {
    fn schema_name() -> String {
        "Direction".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_enum_schema("Direction", &["out", "in", "both"])
    }
}

/// One entry of an edge-kind filter: either a stored kind or a reverse view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeSelector {
    Kind(EdgeKind),
    View(ReverseView),
}

/// Why a kind filter string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown edge kind or reverse view {0:?}")]
pub struct UnknownEdgeKind(pub String);

impl EdgeSelector {
    /// Parses a PRD spelling, accepting both stored kinds and reverse views.
    pub fn parse(raw: &str) -> Result<Self, UnknownEdgeKind> {
        if let Some(kind) = EdgeKind::from_str_exact(raw) {
            return Ok(Self::Kind(kind));
        }
        if let Some(view) = ReverseView::from_str_exact(raw) {
            return Ok(Self::View(view));
        }
        Err(UnknownEdgeKind(raw.to_owned()))
    }

    /// The stored kind plus the direction it must be read in.
    pub const fn underlying(self) -> (EdgeKind, Direction) {
        match self {
            Self::Kind(kind) => (kind, Direction::Out),
            Self::View(view) => (view.underlying(), Direction::In),
        }
    }
}

impl From<EdgeKind> for EdgeSelector {
    fn from(kind: EdgeKind) -> Self {
        Self::Kind(kind)
    }
}

impl From<ReverseView> for EdgeSelector {
    fn from(view: ReverseView) -> Self {
        Self::View(view)
    }
}

impl fmt::Display for EdgeSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kind(kind) => f.write_str(kind.as_str()),
            Self::View(view) => f.write_str(view.as_str()),
        }
    }
}

impl Serialize for EdgeSelector {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl<'de> Deserialize<'de> for EdgeSelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(D::Error::custom)
    }
}

impl JsonSchema for EdgeSelector {
    fn schema_name() -> String {
        "EdgeSelector".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut values: Vec<&str> = EdgeKind::ALL.map(|kind| kind.as_str()).to_vec();
        values.extend(ReverseView::ALL.map(|view| view.as_str()));
        string_enum_schema("EdgeSelector", &values)
    }
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EdgeKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s).ok_or_else(|| format!("EdgeKind has no member {s:?}"))
    }
}

impl Serialize for EdgeKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EdgeKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str_exact(&raw)
            .ok_or_else(|| D::Error::custom(format!("unknown EdgeKind {raw:?}")))
    }
}

impl JsonSchema for EdgeKind {
    fn schema_name() -> String {
        "EdgeKind".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let values = EdgeKind::ALL.map(|kind| kind.as_str());
        string_enum_schema("EdgeKind", &values)
    }
}

/// A `u64` bitset over [`EdgeKind`] discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EdgeKindSet(u64);
impl Serialize for EdgeKindSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for EdgeKindSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u64::deserialize(deserializer)?;
        // Unknown bits are masked the same way `from_bits` masks them, so a hand-written
        // request body cannot smuggle a bit the graph never stores.
        Ok(Self(raw & Self::ALL.0))
    }
}

impl JsonSchema for EdgeKindSet {
    fn schema_name() -> String {
        "EdgeKindSet".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut object = SchemaObject {
            instance_type: Some(InstanceType::Number.into()),
            ..SchemaObject::default()
        };
        object.number = Some(Box::new(NumberValidation {
            minimum: Some(0.0),
            maximum: Some((u64::MAX >> 31) as f64),
            ..NumberValidation::default()
        }));
        Schema::Object(object)
    }
}

impl EdgeKindSet {
    /// No kinds.
    pub const EMPTY: Self = Self(0);
    /// Every stored kind (33 bits).
    pub const ALL: Self = Self(u64::MAX >> (64 - 33));
    /// Repository shape: `CONTAINS`, `DECLARES`, `IMPORTS`, `EXPORTS`, `DEPENDS_ON`.
    pub const STRUCTURAL: Self = Self::of(EdgeKind::Contains)
        .union(Self::of(EdgeKind::Declares))
        .union(Self::of(EdgeKind::Imports))
        .union(Self::of(EdgeKind::Exports))
        .union(Self::of(EdgeKind::DependsOn));
    /// Invocation-like relations: `CALLS`, `THROWS`, `CATCHES`.
    pub const CALL_LIKE: Self = Self::of(EdgeKind::Calls)
        .union(Self::of(EdgeKind::Throws))
        .union(Self::of(EdgeKind::Catches));
    /// Nominal and type relations: `EXTENDS`, `IMPLEMENTS`, `OVERRIDES`, `REFERENCES`,
    /// `USES_TYPE`, `RETURNS_TYPE`, `ACCEPTS_TYPE`.
    pub const TYPE_REL: Self = Self::of(EdgeKind::Extends)
        .union(Self::of(EdgeKind::Implements))
        .union(Self::of(EdgeKind::Overrides))
        .union(Self::of(EdgeKind::References))
        .union(Self::of(EdgeKind::UsesType))
        .union(Self::of(EdgeKind::ReturnsType))
        .union(Self::of(EdgeKind::AcceptsType));
    /// Relations a framework adapter produces: `ROUTES_TO`, `HANDLED_BY`, `TESTS`, `COVERS`,
    /// `PRODUCES_JOB`, `CONSUMES_JOB`, `VALIDATES`, `AUTHORIZES`.
    pub const FRAMEWORK: Self = Self::of(EdgeKind::RoutesTo)
        .union(Self::of(EdgeKind::HandledBy))
        .union(Self::of(EdgeKind::Tests))
        .union(Self::of(EdgeKind::Covers))
        .union(Self::of(EdgeKind::ProducesJob))
        .union(Self::of(EdgeKind::ConsumesJob))
        .union(Self::of(EdgeKind::Validates))
        .union(Self::of(EdgeKind::Authorizes));
    /// Data access: `READS`, `WRITES`, `READS_TABLE`, `WRITES_TABLE`, `READS_CONFIG`,
    /// `WRITES_CONFIG`.
    pub const DATA: Self = Self::of(EdgeKind::Reads)
        .union(Self::of(EdgeKind::Writes))
        .union(Self::of(EdgeKind::ReadsTable))
        .union(Self::of(EdgeKind::WritesTable))
        .union(Self::of(EdgeKind::ReadsConfig))
        .union(Self::of(EdgeKind::WritesConfig));

    /// A set holding exactly `kind`.
    pub const fn of(kind: EdgeKind) -> Self {
        Self(1u64 << kind as u64)
    }

    /// Wraps raw bits. Bits above the 33rd are ignored.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits & Self::ALL.0)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, kind: EdgeKind) -> bool {
        self.0 & (1u64 << kind as u64) != 0
    }

    /// True when any of `other` is present.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub fn insert(&mut self, kind: EdgeKind) {
        self.0 |= 1u64 << kind as u64;
    }

    pub fn remove(&mut self, kind: EdgeKind) {
        self.0 &= !(1u64 << kind as u64);
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The number of kinds held.
    pub const fn len(self) -> usize {
        let n = self.0.count_ones() as usize;
        if n > 33 {
            33
        } else {
            n
        }
    }

    /// Builds a set from an iterator of kinds.
    pub fn from_kinds(kinds: impl IntoIterator<Item = EdgeKind>) -> Self {
        let mut set = Self::EMPTY;
        for kind in kinds {
            set.insert(kind);
        }
        set
    }

    /// Iterates the held kinds in discriminant order.
    pub fn iter(self) -> impl Iterator<Item = EdgeKind> {
        EdgeKind::ALL
            .into_iter()
            .filter(move |kind| self.contains(*kind))
    }
}

impl fmt::Display for EdgeKindSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.iter().map(EdgeKind::as_str).collect();
        write!(f, "[{}]", names.join(","))
    }
}

impl From<EdgeKind> for EdgeKindSet {
    fn from(kind: EdgeKind) -> Self {
        Self::of(kind)
    }
}

impl From<&[EdgeKind]> for EdgeKindSet {
    fn from(kinds: &[EdgeKind]) -> Self {
        Self::from_kinds(kinds.iter().copied())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// PRD §18 lists 35 names: the 33 stored kinds plus `CALLED_BY` and `DEPENDED_ON_BY`.
    /// Gap analysis resolved `TESTED_BY` as a third view, so 36 spellings in total.
    const PRD_18_STORED: [&str; 33] = [
        "CONTAINS",
        "DECLARES",
        "IMPORTS",
        "EXPORTS",
        "CALLS",
        "READS",
        "WRITES",
        "IMPLEMENTS",
        "EXTENDS",
        "OVERRIDES",
        "REFERENCES",
        "USES_TYPE",
        "RETURNS_TYPE",
        "ACCEPTS_TYPE",
        "ROUTES_TO",
        "HANDLED_BY",
        "TESTS",
        "COVERS",
        "PRODUCES_JOB",
        "CONSUMES_JOB",
        "READS_CONFIG",
        "WRITES_CONFIG",
        "READS_TABLE",
        "WRITES_TABLE",
        "DEPENDS_ON",
        "THROWS",
        "CATCHES",
        "SERIALIZES",
        "DESERIALIZES",
        "VALIDATES",
        "AUTHORIZES",
        "PUBLISHES",
        "SUBSCRIBES",
    ];

    const REVERSE_VIEWS: [&str; 3] = ["CALLED_BY", "DEPENDED_ON_BY", "TESTED_BY"];

    #[test]
    fn edge_kind_list_matches_prd_18_minus_reverse_views() {
        let stored: Vec<&str> = ALL_EDGE_KINDS.iter().map(|k| k.as_str()).collect();
        assert_eq!(stored.len(), 33);
        assert_eq!(stored, PRD_18_STORED);
        let views: Vec<&str> = ReverseView::ALL.iter().map(|v| v.as_str()).collect();
        assert_eq!(views, REVERSE_VIEWS);
        assert_eq!(stored.len() + views.len(), 36);
        let mut all = std::collections::HashSet::new();
        for name in stored.into_iter().chain(views) {
            assert!(all.insert(name), "duplicate wire spelling {name}");
        }
    }

    #[test]
    fn edge_kind_has_no_reverse_view_variants() {
        let names: Vec<&str> = EdgeKind::ALL.iter().map(|k| k.as_str()).collect();
        for view in REVERSE_VIEWS {
            assert!(!names.contains(&view), "{view} must not be a stored kind");
        }
        assert!(EdgeKind::from_str_exact("CALLED_BY").is_none());
        assert!(serde_json::from_str::<EdgeKind>("\"CALLED_BY\"").is_err());
        assert_eq!(EdgeKind::from_str_exact("CALLS"), Some(EdgeKind::Calls));
    }

    #[test]
    fn edge_kind_discriminants_snapshot() {
        let pairs: Vec<(&str, u8)> = ALL_EDGE_KINDS
            .iter()
            .map(|k| (k.as_str(), k.as_u8()))
            .collect();
        insta::assert_yaml_snapshot!("edge_kind_discriminants", pairs);
        let mut seen = std::collections::HashSet::new();
        for (_, disc) in &pairs {
            assert!(
                seen.insert(*disc),
                "duplicate edge kind discriminant {disc}"
            );
        }
        assert_eq!(EdgeKind::Contains.as_u8(), 0);
        assert_eq!(EdgeKind::Subscribes.as_u8(), 32);
        assert_eq!(EdgeKind::from_u8(15), Some(EdgeKind::HandledBy));
        assert_eq!(EdgeKind::from_u8(33), None);
    }

    #[test]
    fn reverse_view_maps_to_underlying_kind_in_direction() {
        assert_eq!(ReverseView::CalledBy.underlying(), EdgeKind::Calls);
        assert_eq!(ReverseView::DependedOnBy.underlying(), EdgeKind::DependsOn);
        assert_eq!(ReverseView::TestedBy.underlying(), EdgeKind::Tests);
        assert_eq!(
            EdgeSelector::parse("CALLED_BY").unwrap().underlying(),
            (EdgeKind::Calls, Direction::In)
        );
        assert_eq!(
            EdgeSelector::parse("CALLS").unwrap().underlying(),
            (EdgeKind::Calls, Direction::Out)
        );
        assert_eq!(
            EdgeSelector::parse("TESTED_BY").unwrap().underlying(),
            (EdgeKind::Tests, Direction::In)
        );
        assert_eq!(
            EdgeSelector::parse("TESTED_BY").unwrap().to_string(),
            "TESTED_BY"
        );
        assert_eq!(
            EdgeSelector::parse("nope"),
            Err(UnknownEdgeKind("nope".to_owned()))
        );
        let json = serde_json::to_string(&EdgeSelector::from(EdgeKind::Calls)).unwrap();
        assert_eq!(json, "\"CALLS\"");
        assert_eq!(
            serde_json::from_str::<EdgeSelector>("\"DEPENDED_ON_BY\"").unwrap(),
            EdgeSelector::View(ReverseView::DependedOnBy)
        );
        assert!(serde_json::from_str::<EdgeSelector>("\"NOPE\"").is_err());
    }

    #[test]
    fn edge_kind_set_ops_and_groups() {
        let mut set = EdgeKindSet::EMPTY;
        assert!(set.is_empty());
        set.insert(EdgeKind::Calls);
        assert!(set.contains(EdgeKind::Calls));
        assert!(!set.contains(EdgeKind::Imports));
        set.remove(EdgeKind::Calls);
        assert!(set.is_empty());

        assert_eq!(EdgeKindSet::ALL.len(), 33);
        assert!(EdgeKindSet::ALL.contains(EdgeKind::Subscribes));

        let one = EdgeKindSet::of(EdgeKind::Calls);
        let other = EdgeKindSet::of(EdgeKind::Imports);
        assert!(one.union(other).contains(EdgeKind::Imports));
        assert!(!one.intersect(other).contains(EdgeKind::Imports));
        assert!(one.intersects(other.union(one)));
        assert!(!one.intersects(other));
        assert_eq!(
            EdgeKindSet::from_bits(u64::MAX).bits(),
            EdgeKindSet::ALL.bits()
        );
        assert_eq!(
            EdgeKindSet::from_kinds([EdgeKind::Calls, EdgeKind::Tests]),
            one.union(EdgeKindSet::of(EdgeKind::Tests))
        );
        assert_eq!(EdgeKindSet::from(&[EdgeKind::Calls][..]), one);
        assert_eq!(EdgeKindSet::of(EdgeKind::Calls), one);
        assert_eq!(one.len(), 1);

        assert!(EdgeKindSet::STRUCTURAL.contains(EdgeKind::Imports));
        assert!(!EdgeKindSet::STRUCTURAL.contains(EdgeKind::Calls));
        assert!(EdgeKindSet::CALL_LIKE.contains(EdgeKind::Throws));
        assert!(EdgeKindSet::CALL_LIKE.contains(EdgeKind::Catches));
        assert!(!EdgeKindSet::CALL_LIKE.contains(EdgeKind::Imports));
        assert!(EdgeKindSet::TYPE_REL.contains(EdgeKind::Implements));
        assert!(!EdgeKindSet::TYPE_REL.contains(EdgeKind::Calls));
        assert!(EdgeKindSet::FRAMEWORK.contains(EdgeKind::Authorizes));
        assert!(!EdgeKindSet::FRAMEWORK.contains(EdgeKind::Calls));
        assert!(EdgeKindSet::DATA.contains(EdgeKind::WritesTable));
        assert!(!EdgeKindSet::DATA.contains(EdgeKind::Calls));
        assert!(EdgeKindSet::CALL_LIKE.intersects(EdgeKindSet::ALL));
        assert!(!EdgeKindSet::EMPTY.intersects(EdgeKindSet::CALL_LIKE));
        assert_eq!(format!("{}", EdgeKindSet::of(EdgeKind::Calls)), "[CALLS]");
        assert_eq!(format!("{}", EdgeKindSet::EMPTY), "[]");
        let group: EdgeKindSet = EdgeKind::Calls.into();
        assert_eq!(group, one);
    }

    #[test]
    fn edge_kind_roundtrip_str_and_serde() {
        for kind in EdgeKind::ALL {
            assert_eq!(EdgeKind::from_str_exact(kind.as_str()), Some(kind));
            assert_eq!(kind.to_string(), kind.as_str());
            assert_eq!(kind.as_str().parse::<EdgeKind>().unwrap(), kind);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert_eq!(serde_json::from_str::<EdgeKind>(&json).unwrap(), kind);
            assert_eq!(EdgeKind::from_u8(kind.as_u8()), Some(kind));
        }
        for view in ReverseView::ALL {
            assert_eq!(ReverseView::from_str_exact(view.as_str()), Some(view));
            let json = serde_json::to_string(&view).unwrap();
            assert_eq!(serde_json::from_str::<ReverseView>(&json).unwrap(), view);
            assert_eq!(view.as_str().parse::<ReverseView>().unwrap(), view);
        }
        assert!("nope".parse::<EdgeKind>().is_err());
        assert!("nope".parse::<ReverseView>().is_err());
    }

    #[test]
    fn edge_kind_json_schema_is_prd_spelling() {
        let schema = serde_json::to_value(schemars::schema_for!(EdgeKind)).unwrap();
        assert_eq!(schema["type"], "string");
        let values: Vec<&str> = schema["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(values, PRD_18_STORED);
        let views = serde_json::to_value(schemars::schema_for!(EdgeSelector)).unwrap();
        assert_eq!(views["enum"].as_array().unwrap().len(), 36);
    }

    proptest! {
        #[test]
        fn edge_kind_str_roundtrip(index in 0usize..33) {
            let kind = ALL_EDGE_KINDS[index];
            prop_assert_eq!(EdgeKind::from_str_exact(kind.as_str()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            prop_assert_eq!(serde_json::from_str::<EdgeKind>(&json).unwrap(), kind);
        }
    }
}
