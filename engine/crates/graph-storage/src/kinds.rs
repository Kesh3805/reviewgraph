//! Storage-side mirrors of the graph taxonomy (C4, GS-001).
//!
//! The canonical definitions live in `codegraph` (CG-001/CG-002/CG-003). Those crates are owned
//! by the graph lane; until they land, `graph-storage` keeps the discriminant values, PRD
//! spellings and encodings it persists in `node_kinds`, `edge_kinds`, `resolved_by_kinds` and
//! `provenance_kinds` right here, so the migrations and the Rust side can be compared by a test.
//!
//! Mapping requirement: every type in this module must stay bit-compatible with its
//! `codegraph` counterpart (`NodeKind`, `EdgeKind`, `ResolvedBy`, `Provenance`, `Confidence`,
//! `EdgeKindSet`, `EdgeFlags`). See `docs/graph-schema/storage.md`.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 128-bit node key. The graph lane's `codegraph::NodeKey` is a re-export of this type
/// (CG-001), so storage writes `bytea` of exactly 16 bytes either way.
pub use review_core::ids::SymbolKey as NodeKey;

macro_rules! kind_enum {
    (
        $(#[$meta:meta])*
        $name:ident => $repr:ty, $count:expr,
        variants: [$( ($variant:ident, $disc:expr, $text:literal) ),+ $(,)?]
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr($repr)]
        pub enum $name {
            $( $variant = $disc, )+
        }

        impl $name {
            /// Every variant in discriminant order.
            pub const ALL: [$name; $count] = [ $( Self::$variant, )+ ];

            /// The PRD wire spelling persisted in the `*_kinds` lookup tables.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $text, )+
                }
            }

            pub fn from_str_exact(s: &str) -> Option<Self> {
                match s {
                    $( $text => Some(Self::$variant), )+
                    _ => None,
                }
            }

            /// The discriminant persisted in `smallint` columns.
            pub const fn as_i16(self) -> i16 {
                self as $repr as i16
            }

            pub fn from_i16(raw: i16) -> Option<Self> {
                match raw {
                    $( $disc => Some(Self::$variant), )+
                    _ => None,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::from_str_exact(s)
                    .ok_or_else(|| format!(concat!(stringify!($name), " has no member {:?}"), s))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                Self::from_str_exact(&raw)
                    .ok_or_else(|| D::Error::custom(format!("unknown {} {raw:?}", stringify!($name))))
            }
        }
    };
}

kind_enum! {
    /// Node taxonomy of PRD §17 (44 kinds, clarification C1).
    ///
    /// Gaps between discriminant blocks leave room for language-specific extensions; discarded
    /// discriminants are never reused (CG-011 bumps `SCHEMA_VERSION` instead).
    NodeKind => u8, 44,
    variants: [
        (Repository, 0, "Repository"),
        (Package, 1, "Package"),
        (Module, 2, "Module"),
        (Directory, 3, "Directory"),
        (File, 4, "File"),
        (Namespace, 10, "Namespace"),
        (Class, 11, "Class"),
        (Interface, 12, "Interface"),
        (Struct, 13, "Struct"),
        (Trait, 14, "Trait"),
        (Enum, 15, "Enum"),
        (TypeAlias, 16, "TypeAlias"),
        (Function, 20, "Function"),
        (Method, 21, "Method"),
        (Constructor, 22, "Constructor"),
        (Property, 23, "Property"),
        (Field, 24, "Field"),
        (Parameter, 25, "Parameter"),
        (Variable, 26, "Variable"),
        (Constant, 27, "Constant"),
        (ApiEndpoint, 30, "ApiEndpoint"),
        (Controller, 31, "Controller"),
        (Handler, 32, "Handler"),
        (Middleware, 33, "Middleware"),
        (DatabaseEntity, 40, "DatabaseEntity"),
        (DatabaseTable, 41, "DatabaseTable"),
        (DatabaseColumn, 42, "DatabaseColumn"),
        (Migration, 43, "Migration"),
        (Queue, 50, "Queue"),
        (QueueProducer, 51, "QueueProducer"),
        (QueueConsumer, 52, "QueueConsumer"),
        (JobHandler, 53, "JobHandler"),
        (Configuration, 60, "Configuration"),
        (EnvironmentVariable, 61, "EnvironmentVariable"),
        (TestSuite, 70, "TestSuite"),
        (TestCase, 71, "TestCase"),
        (Fixture, 72, "Fixture"),
        (ExternalDependency, 80, "ExternalDependency"),
        (ExternalApi, 81, "ExternalApi"),
        (BuildTarget, 90, "BuildTarget"),
        (CliCommand, 91, "CliCommand"),
        (Worker, 92, "Worker"),
        (DocumentationRule, 100, "DocumentationRule"),
        (ArchitecturalBoundary, 101, "ArchitecturalBoundary"),
    ]
}

kind_enum! {
    /// Stored edge taxonomy of PRD §18 (33 kinds; the reverse views are never stored, C2).
    EdgeKind => u8, 33,
    variants: [
        (Contains, 0, "CONTAINS"),
        (Declares, 1, "DECLARES"),
        (Imports, 2, "IMPORTS"),
        (Exports, 3, "EXPORTS"),
        (Calls, 4, "CALLS"),
        (Reads, 5, "READS"),
        (Writes, 6, "WRITES"),
        (Implements, 7, "IMPLEMENTS"),
        (Extends, 8, "EXTENDS"),
        (Overrides, 9, "OVERRIDES"),
        (References, 10, "REFERENCES"),
        (UsesType, 11, "USES_TYPE"),
        (ReturnsType, 12, "RETURNS_TYPE"),
        (AcceptsType, 13, "ACCEPTS_TYPE"),
        (RoutesTo, 14, "ROUTES_TO"),
        (HandledBy, 15, "HANDLED_BY"),
        (Tests, 16, "TESTS"),
        (Covers, 17, "COVERS"),
        (ProducesJob, 18, "PRODUCES_JOB"),
        (ConsumesJob, 19, "CONSUMES_JOB"),
        (ReadsConfig, 20, "READS_CONFIG"),
        (WritesConfig, 21, "WRITES_CONFIG"),
        (ReadsTable, 22, "READS_TABLE"),
        (WritesTable, 23, "WRITES_TABLE"),
        (DependsOn, 24, "DEPENDS_ON"),
        (Throws, 25, "THROWS"),
        (Catches, 26, "CATCHES"),
        (Serializes, 27, "SERIALIZES"),
        (Deserializes, 28, "DESERIALIZES"),
        (Validates, 29, "VALIDATES"),
        (Authorizes, 30, "AUTHORIZES"),
        (Publishes, 31, "PUBLISHES"),
        (Subscribes, 32, "SUBSCRIBES"),
    ]
}

kind_enum! {
    /// How a reference was resolved to an edge (CG-003 keys the confidence table by it).
    ResolvedBy => u8, 10,
    variants: [
        (Structural, 0, "STRUCTURAL"),
        (Import, 1, "IMPORT"),
        (ThisMember, 2, "THIS_MEMBER"),
        (DiConstructor, 3, "DI_CONSTRUCTOR"),
        (TypeAnnotation, 4, "TYPE_ANNOTATION"),
        (NameUnique, 5, "NAME_UNIQUE"),
        (NameAmbiguous, 6, "NAME_AMBIGUOUS"),
        (Framework, 7, "FRAMEWORK"),
        (TypeChecker, 8, "TYPE_CHECKER"),
        (Heuristic, 9, "HEURISTIC"),
    ]
}

kind_enum! {
    /// Which stage produced an edge row.
    Provenance => u8, 6,
    variants: [
        (Analyzer, 0, "ANALYZER"),
        (Framework, 1, "FRAMEWORK"),
        (Linker, 2, "LINKER"),
        (TypeChecker, 3, "TYPE_CHECKER"),
        (Heuristic, 4, "HEURISTIC"),
        (Policy, 5, "POLICY"),
    ]
}

/// Edge confidence in permille, 0..=1000 (CG-003 owns the value table).
///
/// Persisted as `real` in `graph_edges.confidence` (`permille / 1000`); the conversion is
/// exact for every integer permille value, which the storage round-trip test pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Confidence(u16);

impl Confidence {
    pub const MIN: Self = Self(0);
    pub const MAX: Self = Self(1000);

    pub const fn from_permille(permille: u16) -> Self {
        Self(if permille > 1000 { 1000 } else { permille })
    }

    pub const fn as_permille(self) -> u16 {
        self.0
    }

    pub fn from_f32(value: f32) -> Self {
        if !value.is_finite() || value <= 0.0 {
            return Self::MIN;
        }
        if value >= 1.0 {
            return Self::MAX;
        }
        Self((value * 1000.0).round() as u16)
    }

    pub fn as_f32(self) -> f32 {
        f32::from(self.0) / 1000.0
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}‰", self.0)
    }
}

impl Serialize for Confidence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for Confidence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u16::deserialize(deserializer)?;
        if raw > 1000 {
            return Err(D::Error::custom(format!("confidence {raw} > 1000")));
        }
        Ok(Self(raw))
    }
}

impl Serialize for EdgeFlags {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(u16::from(self.0))
    }
}

impl<'de> Deserialize<'de> for EdgeFlags {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u16::deserialize(deserializer)?;
        if raw > u16::from(u8::MAX) {
            return Err(D::Error::custom(format!("edge flags {raw} > 255")));
        }
        Ok(Self(raw as u8))
    }
}

/// Bit flags carried by an edge row (`graph_edges.flags`, a `smallint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct EdgeFlags(u8);

impl EdgeFlags {
    pub const INSTANTIATES: Self = Self(1);
    pub const DECORATOR: Self = Self(2);
    pub const TYPE_ONLY: Self = Self(4);
    pub const DYNAMIC: Self = Self(8);
    pub const MAPS_TABLE: Self = Self(16);
    pub const GLOBAL_SCOPE: Self = Self(32);
    pub const EMPTY: Self = Self(0);

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
}

impl std::ops::BitOr for EdgeFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

/// Direction of a neighbour lookup: `Out` follows `source -> target`, `In` the reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Out,
    In,
}

impl Direction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Out => "out",
            Self::In => "in",
        }
    }
}

/// A `u64` bitset over [`EdgeKind`] discriminants (CG-002 `EdgeKindSet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EdgeKindSet(u64);

impl EdgeKindSet {
    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self(u64::MAX >> (64 - 33));

    pub const fn of(kind: EdgeKind) -> Self {
        Self(1u64 << kind as u64)
    }

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, kind: EdgeKind) -> bool {
        self.0 & (1u64 << kind as u64) != 0
    }

    pub fn insert(&mut self, kind: EdgeKind) {
        self.0 |= 1u64 << kind as u64;
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

    pub fn from_kinds(kinds: impl IntoIterator<Item = EdgeKind>) -> Self {
        let mut set = Self::EMPTY;
        for kind in kinds {
            set.insert(kind);
        }
        set
    }

    pub fn iter(self) -> impl Iterator<Item = EdgeKind> {
        EdgeKind::ALL.into_iter().filter(move |k| self.contains(*k))
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn node_kind_list_matches_prd_17() {
        let names: Vec<&str> = NodeKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(names.len(), 44);
        assert_eq!(names[0], "Repository");
        assert_eq!(names[43], "ArchitecturalBoundary");
        assert!(names.contains(&"ApiEndpoint"));
        assert!(names.contains(&"CliCommand"));
        assert!(!names.contains(&"CLICommand"));
    }

    #[test]
    fn edge_kind_list_has_33_stored_kinds_and_no_reverse_views() {
        assert_eq!(EdgeKind::ALL.len(), 33);
        for kind in EdgeKind::ALL {
            assert!(!kind.as_str().contains("CALLED_BY"));
            assert!(!kind.as_str().contains("DEPENDED_ON_BY"));
            assert!(!kind.as_str().contains("TESTED_BY"));
        }
    }

    #[test]
    fn discriminants_are_unique_and_stable() {
        let mut seen = std::collections::HashSet::new();
        for kind in NodeKind::ALL {
            assert!(
                seen.insert(kind.as_i16()),
                "duplicate node kind discriminant"
            );
        }
        assert_eq!(NodeKind::Repository.as_i16(), 0);
        assert_eq!(NodeKind::ArchitecturalBoundary.as_i16(), 101);
        let mut seen = std::collections::HashSet::new();
        for kind in EdgeKind::ALL {
            assert!(
                seen.insert(kind.as_i16()),
                "duplicate edge kind discriminant"
            );
        }
        assert_eq!(EdgeKind::Contains.as_i16(), 0);
        assert_eq!(EdgeKind::Subscribes.as_i16(), 32);
    }

    #[test]
    fn kind_roundtrips_through_str_and_serde() {
        for kind in NodeKind::ALL {
            assert_eq!(NodeKind::from_str_exact(kind.as_str()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<NodeKind>(&json).unwrap(), kind);
        }
        for kind in EdgeKind::ALL {
            assert_eq!(EdgeKind::from_str_exact(kind.as_str()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<EdgeKind>(&json).unwrap(), kind);
        }
        for kind in ResolvedBy::ALL {
            assert_eq!(ResolvedBy::from_i16(kind.as_i16()), Some(kind));
        }
        for kind in Provenance::ALL {
            assert_eq!(Provenance::from_i16(kind.as_i16()), Some(kind));
        }
        assert!(serde_json::from_str::<EdgeKind>("\"NOPE\"").is_err());
    }

    #[test]
    fn confidence_converts_exactly_through_f32() {
        for permille in 0..=1000u16 {
            let c = Confidence::from_permille(permille);
            assert_eq!(Confidence::from_f32(c.as_f32()).as_permille(), permille);
        }
        assert_eq!(Confidence::from_f32(0.95).as_permille(), 950);
        assert_eq!(Confidence::from_f32(0.3).as_permille(), 300);
        assert_eq!(Confidence::from_f32(f32::NAN).as_permille(), 0);
        assert_eq!(Confidence::from_f32(1.5).as_permille(), 1000);
    }

    #[test]
    fn edge_kind_set_covers_all_kinds() {
        assert_eq!(EdgeKindSet::ALL.iter().count(), 33);
        assert!(EdgeKindSet::ALL.contains(EdgeKind::Subscribes));
        assert!(!EdgeKindSet::EMPTY.contains(EdgeKind::Calls));
        let one = EdgeKindSet::of(EdgeKind::Calls);
        assert!(one.contains(EdgeKind::Calls));
        assert!(!one.contains(EdgeKind::Imports));
        assert!(one
            .union(EdgeKindSet::of(EdgeKind::Imports))
            .contains(EdgeKind::Imports));
        assert!(!one
            .intersect(EdgeKindSet::of(EdgeKind::Imports))
            .contains(EdgeKind::Imports));
    }

    #[test]
    fn edge_flags_union_and_contains() {
        let flags = EdgeFlags::DYNAMIC | EdgeFlags::TYPE_ONLY;
        assert!(flags.contains(EdgeFlags::DYNAMIC));
        assert!(flags.contains(EdgeFlags::TYPE_ONLY));
        assert!(!flags.contains(EdgeFlags::DECORATOR));
        assert!(EdgeFlags::EMPTY.is_empty());
        assert_eq!(flags.bits(), 12);
    }
}
