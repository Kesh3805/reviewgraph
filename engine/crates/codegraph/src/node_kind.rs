//! Node taxonomy (CG-001): PRD §17's 44 [`NodeKind`]s, their [`NodeCategory`] and the
//! refinement rules the framework mapper applies (clarification C3).
//!
//! Discriminants are stable `u8`s. The gaps between the blocks leave room for
//! language-specific extensions (PRD §17, last line) without renumbering; a discarded
//! discriminant is never reused and removing a kind requires a `SCHEMA_VERSION` bump
//! (CG-011). The PRD spelling used by `as_str` is the wire form stored in the
//! `node_kinds` lookup table and the form `packages/contracts` sees.

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use review_core::symbol::SymbolKind;

use crate::schema_util::string_enum_schema;

/// Twelve coarse buckets over the node taxonomy, used by [`crate::schema_rules`] and by the
/// validation warnings of CG-012.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NodeCategory {
    /// Repository shape: repositories, packages, modules, directories, files, namespaces.
    Structural,
    /// Nominal types: classes, interfaces, structs, traits, enums, type aliases.
    Type,
    /// Things that can be called: functions, methods, constructors.
    Callable,
    /// Value carriers: properties, fields, parameters, variables, constants.
    Data,
    /// Web/runtime framework constructs: endpoints, controllers, handlers, middleware.
    Framework,
    /// Tables, entities, columns and migrations.
    Database,
    /// Queues, producers, consumers and job handlers.
    Messaging,
    /// Configuration and environment variables.
    Config,
    /// Test suites, test cases and fixtures.
    Test,
    /// Dependencies and APIs outside the repository.
    External,
    /// Build targets, CLI commands and workers.
    Build,
    /// Documentation rules and architectural boundaries.
    Governance,
}

impl NodeCategory {
    /// Every category, in declaration order.
    pub const ALL: [NodeCategory; 12] = [
        NodeCategory::Structural,
        NodeCategory::Type,
        NodeCategory::Callable,
        NodeCategory::Data,
        NodeCategory::Framework,
        NodeCategory::Database,
        NodeCategory::Messaging,
        NodeCategory::Config,
        NodeCategory::Test,
        NodeCategory::External,
        NodeCategory::Build,
        NodeCategory::Governance,
    ];
}

/// The node kinds of PRD §17. Exactly 44 variants, including `Repository` (clarification C1:
/// the "46 + Repository" count in the target architecture is a miscount).
///
/// `repr(u8)` discriminants are part of the persisted wire format; they are compared against
/// the `node_kinds` lookup table by `graph-storage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum NodeKind {
    Repository = 0,
    Package = 1,
    Module = 2,
    Directory = 3,
    File = 4,
    Namespace = 10,
    Class = 11,
    Interface = 12,
    Struct = 13,
    Trait = 14,
    Enum = 15,
    TypeAlias = 16,
    Function = 20,
    Method = 21,
    Constructor = 22,
    Property = 23,
    Field = 24,
    Parameter = 25,
    Variable = 26,
    Constant = 27,
    ApiEndpoint = 30,
    Controller = 31,
    Handler = 32,
    Middleware = 33,
    DatabaseEntity = 40,
    DatabaseTable = 41,
    DatabaseColumn = 42,
    Migration = 43,
    Queue = 50,
    QueueProducer = 51,
    QueueConsumer = 52,
    JobHandler = 53,
    Configuration = 60,
    EnvironmentVariable = 61,
    TestSuite = 70,
    TestCase = 71,
    Fixture = 72,
    ExternalDependency = 80,
    ExternalApi = 81,
    BuildTarget = 90,
    CliCommand = 91,
    Worker = 92,
    DocumentationRule = 100,
    ArchitecturalBoundary = 101,
}

/// Every node kind, in discriminant order.
pub const ALL_NODE_KINDS: [NodeKind; 44] = [
    NodeKind::Repository,
    NodeKind::Package,
    NodeKind::Module,
    NodeKind::Directory,
    NodeKind::File,
    NodeKind::Namespace,
    NodeKind::Class,
    NodeKind::Interface,
    NodeKind::Struct,
    NodeKind::Trait,
    NodeKind::Enum,
    NodeKind::TypeAlias,
    NodeKind::Function,
    NodeKind::Method,
    NodeKind::Constructor,
    NodeKind::Property,
    NodeKind::Field,
    NodeKind::Parameter,
    NodeKind::Variable,
    NodeKind::Constant,
    NodeKind::ApiEndpoint,
    NodeKind::Controller,
    NodeKind::Handler,
    NodeKind::Middleware,
    NodeKind::DatabaseEntity,
    NodeKind::DatabaseTable,
    NodeKind::DatabaseColumn,
    NodeKind::Migration,
    NodeKind::Queue,
    NodeKind::QueueProducer,
    NodeKind::QueueConsumer,
    NodeKind::JobHandler,
    NodeKind::Configuration,
    NodeKind::EnvironmentVariable,
    NodeKind::TestSuite,
    NodeKind::TestCase,
    NodeKind::Fixture,
    NodeKind::ExternalDependency,
    NodeKind::ExternalApi,
    NodeKind::BuildTarget,
    NodeKind::CliCommand,
    NodeKind::Worker,
    NodeKind::DocumentationRule,
    NodeKind::ArchitecturalBoundary,
];

impl NodeKind {
    /// Every variant, in discriminant order.
    pub const ALL: [NodeKind; 44] = ALL_NODE_KINDS;

    /// The PRD spelling. This is the persisted wire form and the JSON Schema value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "Repository",
            Self::Package => "Package",
            Self::Module => "Module",
            Self::Directory => "Directory",
            Self::File => "File",
            Self::Namespace => "Namespace",
            Self::Class => "Class",
            Self::Interface => "Interface",
            Self::Struct => "Struct",
            Self::Trait => "Trait",
            Self::Enum => "Enum",
            Self::TypeAlias => "TypeAlias",
            Self::Function => "Function",
            Self::Method => "Method",
            Self::Constructor => "Constructor",
            Self::Property => "Property",
            Self::Field => "Field",
            Self::Parameter => "Parameter",
            Self::Variable => "Variable",
            Self::Constant => "Constant",
            Self::ApiEndpoint => "ApiEndpoint",
            Self::Controller => "Controller",
            Self::Handler => "Handler",
            Self::Middleware => "Middleware",
            Self::DatabaseEntity => "DatabaseEntity",
            Self::DatabaseTable => "DatabaseTable",
            Self::DatabaseColumn => "DatabaseColumn",
            Self::Migration => "Migration",
            Self::Queue => "Queue",
            Self::QueueProducer => "QueueProducer",
            Self::QueueConsumer => "QueueConsumer",
            Self::JobHandler => "JobHandler",
            Self::Configuration => "Configuration",
            Self::EnvironmentVariable => "EnvironmentVariable",
            Self::TestSuite => "TestSuite",
            Self::TestCase => "TestCase",
            Self::Fixture => "Fixture",
            Self::ExternalDependency => "ExternalDependency",
            Self::ExternalApi => "ExternalApi",
            Self::BuildTarget => "BuildTarget",
            Self::CliCommand => "CliCommand",
            Self::Worker => "Worker",
            Self::DocumentationRule => "DocumentationRule",
            Self::ArchitecturalBoundary => "ArchitecturalBoundary",
        }
    }

    /// Parses the PRD spelling. `None` for anything else.
    pub fn from_str_exact(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == s)
    }

    /// The discriminant persisted in the `node_kinds.smallint` column.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Reads a persisted discriminant back. `None` for unknown values.
    pub fn from_u8(raw: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_u8() == raw)
    }

    /// The coarse category this kind belongs to.
    pub const fn category(self) -> NodeCategory {
        match self {
            Self::Repository
            | Self::Package
            | Self::Module
            | Self::Directory
            | Self::File
            | Self::Namespace => NodeCategory::Structural,
            Self::Class
            | Self::Interface
            | Self::Struct
            | Self::Trait
            | Self::Enum
            | Self::TypeAlias => NodeCategory::Type,
            Self::Function | Self::Method | Self::Constructor => NodeCategory::Callable,
            Self::Property | Self::Field | Self::Parameter | Self::Variable | Self::Constant => {
                NodeCategory::Data
            }
            Self::ApiEndpoint | Self::Controller | Self::Handler | Self::Middleware => {
                NodeCategory::Framework
            }
            Self::DatabaseEntity | Self::DatabaseTable | Self::DatabaseColumn | Self::Migration => {
                NodeCategory::Database
            }
            Self::Queue | Self::QueueProducer | Self::QueueConsumer | Self::JobHandler => {
                NodeCategory::Messaging
            }
            Self::Configuration | Self::EnvironmentVariable => NodeCategory::Config,
            Self::TestSuite | Self::TestCase | Self::Fixture => NodeCategory::Test,
            Self::ExternalDependency | Self::ExternalApi => NodeCategory::External,
            Self::BuildTarget | Self::CliCommand | Self::Worker => NodeCategory::Build,
            Self::DocumentationRule | Self::ArchitecturalBoundary => NodeCategory::Governance,
        }
    }

    /// Syntactic kinds a framework fact may refine **into** this kind (clarification C3).
    ///
    /// Refinement changes the node's displayed kind only: the `SymbolId` kind segment keeps
    /// the IR syntactic kind, so the [`crate::NodeKey`] never changes.
    pub const fn refinable_from(self) -> &'static [NodeKind] {
        match self {
            Self::Middleware => &[NodeKind::Class, NodeKind::Function],
            Self::Controller => &[NodeKind::Class],
            Self::DatabaseEntity => &[NodeKind::Class],
            Self::JobHandler => &[NodeKind::Method, NodeKind::Function],
            Self::QueueConsumer => &[NodeKind::Class],
            Self::CliCommand => &[NodeKind::Class, NodeKind::Function],
            Self::Worker => &[NodeKind::Class],
            Self::Handler => &[NodeKind::Function],
            _ => &[],
        }
    }

    /// True when a node whose current kind is `from` may be refined into `self`.
    pub const fn refines_from(self, from: NodeKind) -> bool {
        let allowed = self.refinable_from();
        let mut i = 0;
        while i < allowed.len() {
            if allowed[i] as u8 == from as u8 {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Maps a syntactic [`SymbolKind`] from `analysis-ir` onto the graph taxonomy.
    ///
    /// Enum members map to [`NodeKind::Constant`] and getters/setters to
    /// [`NodeKind::Property`]; the analyzer records `enum_member` as a node attribute.
    pub const fn from_ir_symbol_kind(kind: SymbolKind) -> NodeKind {
        match kind {
            SymbolKind::Module => NodeKind::Module,
            SymbolKind::Namespace => NodeKind::Namespace,
            SymbolKind::Class => NodeKind::Class,
            SymbolKind::Interface => NodeKind::Interface,
            SymbolKind::Enum => NodeKind::Enum,
            SymbolKind::EnumMember => NodeKind::Constant,
            SymbolKind::TypeAlias => NodeKind::TypeAlias,
            SymbolKind::Function => NodeKind::Function,
            SymbolKind::Method => NodeKind::Method,
            SymbolKind::Getter | SymbolKind::Setter => NodeKind::Property,
            SymbolKind::Constructor => NodeKind::Constructor,
            SymbolKind::Property => NodeKind::Property,
            SymbolKind::Field => NodeKind::Field,
            SymbolKind::Variable => NodeKind::Variable,
            SymbolKind::Constant => NodeKind::Constant,
            SymbolKind::Parameter => NodeKind::Parameter,
        }
    }

    /// True for kinds that only ever exist as synthetic nodes (never produced from a
    /// declaration). Used by the CG-012 orphan warning.
    pub const fn is_synthetic_only(self) -> bool {
        matches!(
            self,
            Self::Repository
                | Self::Directory
                | Self::ApiEndpoint
                | Self::DatabaseTable
                | Self::DatabaseColumn
                | Self::Queue
                | Self::EnvironmentVariable
                | Self::TestSuite
                | Self::TestCase
                | Self::ExternalDependency
                | Self::Fixture
        )
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for NodeKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s).ok_or_else(|| format!("NodeKind has no member {s:?}"))
    }
}

impl Serialize for NodeKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NodeKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str_exact(&raw)
            .ok_or_else(|| D::Error::custom(format!("unknown NodeKind {raw:?}")))
    }
}

impl JsonSchema for NodeKind {
    fn schema_name() -> String {
        "NodeKind".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let values = NodeKind::ALL.map(|kind| kind.as_str());
        string_enum_schema("NodeKind", &values)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// PRD §17, in order, exactly as spelled there (clarification C1 pins the count at 44).
    const PRD_17: [&str; 44] = [
        "Repository",
        "Package",
        "Module",
        "Directory",
        "File",
        "Namespace",
        "Class",
        "Interface",
        "Struct",
        "Trait",
        "Enum",
        "TypeAlias",
        "Function",
        "Method",
        "Constructor",
        "Property",
        "Field",
        "Parameter",
        "Variable",
        "Constant",
        "ApiEndpoint",
        "Controller",
        "Handler",
        "Middleware",
        "DatabaseEntity",
        "DatabaseTable",
        "DatabaseColumn",
        "Migration",
        "Queue",
        "QueueProducer",
        "QueueConsumer",
        "JobHandler",
        "Configuration",
        "EnvironmentVariable",
        "TestSuite",
        "TestCase",
        "Fixture",
        "ExternalDependency",
        "ExternalApi",
        "BuildTarget",
        "CliCommand",
        "Worker",
        "DocumentationRule",
        "ArchitecturalBoundary",
    ];

    #[test]
    fn node_kind_list_matches_prd_17() {
        let listed: Vec<&str> = ALL_NODE_KINDS.iter().map(|k| k.as_str()).collect();
        assert_eq!(listed.len(), 44);
        assert_eq!(listed, PRD_17);
    }

    #[test]
    fn node_kind_discriminants_are_unique_and_stable() {
        let pairs: Vec<(&str, u8)> = ALL_NODE_KINDS
            .iter()
            .map(|k| (k.as_str(), k.as_u8()))
            .collect();
        let mut seen = std::collections::HashSet::new();
        for (_, disc) in &pairs {
            assert!(
                seen.insert(*disc),
                "duplicate node kind discriminant {disc}"
            );
        }
        insta::assert_yaml_snapshot!("node_kind_discriminants", pairs);
        assert_eq!(NodeKind::Repository.as_u8(), 0);
        assert_eq!(NodeKind::ArchitecturalBoundary.as_u8(), 101);
        assert_eq!(NodeKind::from_u8(43), Some(NodeKind::Migration));
        assert_eq!(NodeKind::from_u8(99), None);
    }

    #[test]
    fn node_kind_roundtrip_str_and_serde() {
        for kind in NodeKind::ALL {
            assert_eq!(NodeKind::from_str_exact(kind.as_str()), Some(kind));
            assert_eq!(kind.to_string(), kind.as_str());
            assert_eq!(kind.as_str().parse::<NodeKind>().unwrap(), kind);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert_eq!(serde_json::from_str::<NodeKind>(&json).unwrap(), kind);
            assert_eq!(NodeKind::from_u8(kind.as_u8()), Some(kind));
        }
        assert!(serde_json::from_str::<NodeKind>("\"Nope\"").is_err());
        assert!("nope".parse::<NodeKind>().is_err());
    }

    #[test]
    fn node_kind_categories_cover_every_kind() {
        let mut counts = std::collections::BTreeMap::new();
        for kind in NodeKind::ALL {
            *counts.entry(kind.category()).or_insert(0usize) += 1;
        }
        assert_eq!(counts.len(), 12);
        let total: usize = counts.values().sum();
        assert_eq!(total, 44);
        assert_eq!(NodeKind::Repository.category(), NodeCategory::Structural);
        assert_eq!(NodeKind::ApiEndpoint.category(), NodeCategory::Framework);
        assert_eq!(NodeKind::Worker.category(), NodeCategory::Build);
    }

    #[test]
    fn refinement_sources_match_the_framework_contract() {
        assert!(NodeKind::Middleware.refines_from(NodeKind::Class));
        assert!(NodeKind::Middleware.refines_from(NodeKind::Function));
        assert!(!NodeKind::Middleware.refines_from(NodeKind::Method));
        assert!(NodeKind::Controller.refines_from(NodeKind::Class));
        assert!(!NodeKind::Controller.refines_from(NodeKind::Interface));
        assert!(NodeKind::JobHandler.refines_from(NodeKind::Method));
        assert!(NodeKind::QueueConsumer.refines_from(NodeKind::Class));
        assert!(NodeKind::DatabaseEntity.refines_from(NodeKind::Class));
        assert!(NodeKind::CliCommand.refines_from(NodeKind::Function));
        assert!(NodeKind::Worker.refines_from(NodeKind::Class));
        assert!(NodeKind::Handler.refines_from(NodeKind::Function));
        assert!(!NodeKind::Class.refines_from(NodeKind::Class));
        assert!(NodeKind::ApiEndpoint.refinable_from().is_empty());
    }

    #[test]
    fn from_ir_symbol_kind_maps_syntactic_kinds() {
        use SymbolKind::*;
        assert_eq!(NodeKind::from_ir_symbol_kind(Class), NodeKind::Class);
        assert_eq!(
            NodeKind::from_ir_symbol_kind(EnumMember),
            NodeKind::Constant
        );
        assert_eq!(NodeKind::from_ir_symbol_kind(Getter), NodeKind::Property);
        assert_eq!(NodeKind::from_ir_symbol_kind(Setter), NodeKind::Property);
        assert_eq!(
            NodeKind::from_ir_symbol_kind(TypeAlias),
            NodeKind::TypeAlias
        );
        assert_eq!(
            NodeKind::from_ir_symbol_kind(Parameter),
            NodeKind::Parameter
        );
        let mut mapped = 0;
        for kind in SymbolKind::ALL {
            let node = NodeKind::from_ir_symbol_kind(kind);
            assert!(NodeKind::ALL.contains(&node), "{kind:?} -> {node:?}");
            mapped += 1;
        }
        assert_eq!(mapped, SymbolKind::ALL.len());
    }

    #[test]
    fn json_schema_is_a_closed_string_enum_in_prd_spelling() {
        let schema = serde_json::to_value(schemars::schema_for!(NodeKind)).unwrap();
        assert_eq!(schema["type"], "string");
        let values: Vec<String> = schema["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(values.len(), 44);
        assert_eq!(values, PRD_17);
    }

    proptest! {
        #[test]
        fn node_kind_str_roundtrip(index in 0usize..44) {
            let kind = ALL_NODE_KINDS[index];
            prop_assert_eq!(NodeKind::from_str_exact(kind.as_str()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            prop_assert_eq!(serde_json::from_str::<NodeKind>(&json).unwrap(), kind);
        }
    }
}
