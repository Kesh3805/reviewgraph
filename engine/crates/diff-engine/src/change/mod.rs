//! The change model (CHG-001..CHG-008): changed symbols with both versions, deterministic change
//! classes from syntax-fact comparison, aggregate collectors and the assembled
//! [`PullRequestChangeModel`].
//!
//! * [`symbols`] / [`versions`] — `ChangedSymbol` from the symbol map and both units (CHG-001).
//! * [`fact_diff`] / [`classify`] / [`classes`] — fact comparison and the 16 classes (CHG-002..005).
//! * [`bind`] / [`graph`] — call binding against base/head graphs (CHG-003).
//! * [`policy`] — the classifier policy tables (CHG-005).
//! * [`collect`] — APIs, dependencies, schemas, configuration and tests (CHG-006).
//! * [`model`] / [`assemble`] — the versioned, hashable model (CHG-007).

pub mod assemble;
pub mod bind;
pub mod classes;
pub mod classify;
pub mod collect;
pub mod fact_diff;
pub mod graph;
pub mod model;
pub mod policy;
pub mod symbols;
pub mod versions;

use std::collections::BTreeMap;
use std::fmt;

use analysis_ir::facts::FactKind;
use review_core::change::SymbolChange;
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::{DiffSide, RepoPath, SourceRange};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use classify::{classify_symbols, default_classifiers, ClassifyCx, Classifier};
pub use fact_diff::{diff_facts, FactDiff};
pub use graph::{GraphPair, GraphView};
pub use model::{ChangeModel, PullRequestChangeModel, CHANGE_MODEL_VERSION};
pub use policy::ClassifierPolicy;
pub use symbols::{build_changed_symbols, ChangeCfg, ChangedSymbols};
pub use versions::SymbolVersion;

/// How a symbol changed. Mirrors [`review_core::change::SymbolChange`] plus `Cosmetic` for
/// symbols whose hashes are unchanged (comment-only or formatting edits).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeKind {
    /// Not in base.
    Added,
    /// Not in head (and not renamed).
    Removed,
    /// Present on both sides with these hashes differing.
    Modified {
        /// `signature_hash` differs.
        signature: bool,
        /// `body_hash` differs.
        body: bool,
        /// `attr_hash` differs.
        attrs: bool,
    },
    /// Renamed or moved (lineage), possibly also edited.
    Renamed {
        /// Base-side id.
        from: SymbolId,
        /// Matcher similarity in `[0, 1]`.
        similarity: f32,
    },
    /// Touched by the diff with all hashes unchanged.
    Cosmetic,
}

impl ChangeKind {
    /// Stable metric/summary label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Modified { .. } => "modified",
            Self::Renamed { .. } => "renamed",
            Self::Cosmetic => "cosmetic",
        }
    }

    /// The DOM-005 change, `None` for cosmetic records.
    pub fn to_core(&self) -> Option<SymbolChange> {
        match self {
            Self::Added => Some(SymbolChange::Added),
            Self::Removed => Some(SymbolChange::Removed),
            Self::Modified {
                signature,
                body,
                attrs,
            } => SymbolChange::modified(*signature, *body, *attrs).ok(),
            Self::Renamed { from, similarity } => {
                SymbolChange::renamed(from.clone(), *similarity).ok()
            }
            Self::Cosmetic => None,
        }
    }
}

/// Symbol flags (CHG-001).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolFlags(u8);

impl SymbolFlags {
    /// In a generated file.
    pub const GENERATED: Self = Self(1);
    /// In a test file.
    pub const TEST: Self = Self(2);
    /// Exported from its module.
    pub const EXPORTED: Self = Self(4);
    /// Part of the public API (exported, or a public member of an exported type).
    pub const PUBLIC_API: Self = Self(8);
    /// An entrypoint (HTTP handler, queue consumer, scheduled job).
    pub const ENTRYPOINT: Self = Self(16);

    const NAMES: [(Self, &'static str); 5] = [
        (Self::GENERATED, "generated"),
        (Self::TEST, "test"),
        (Self::EXPORTED, "exported"),
        (Self::PUBLIC_API, "public_api"),
        (Self::ENTRYPOINT, "entrypoint"),
    ];

    /// Whether every bit of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Set the bits of `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Names of the set flags, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(f, _)| self.contains(*f))
            .map(|(_, n)| *n)
            .collect()
    }
}

impl Serialize for SymbolFlags {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.names().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SymbolFlags {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        let mut flags = Self::default();
        for name in names {
            let Some((flag, _)) = Self::NAMES.iter().find(|(_, n)| *n == name) else {
                return Err(serde::de::Error::custom(format!("unknown symbol flag {name}")));
            };
            flags.insert(*flag);
        }
        Ok(flags)
    }
}

impl JsonSchema for SymbolFlags {
    fn schema_name() -> String {
        "SymbolFlags".to_owned()
    }

    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        Vec::<String>::json_schema(gen)
    }
}

/// The 16 change classes (PRD §28 plus `loop_changed`). Wire strings are the PRD names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChangeClass {
    /// Branch points inserted, removed or reordered.
    ControlFlowChanged,
    /// A condition expression changed, appeared or disappeared.
    ConditionChanged,
    /// A loop changed, appeared or disappeared.
    LoopChanged,
    /// The set of returned expressions changed.
    ReturnChanged,
    /// Try/catch/finally/throw behaviour changed.
    ExceptionHandlingChanged,
    /// Await/async/promise behaviour changed.
    AsyncBehaviorChanged,
    /// A call was added.
    CallAdded,
    /// A call was removed.
    CallRemoved,
    /// The symbol now depends on a new class, module or package.
    DependencyAdded,
    /// The symbol no longer depends on a class, module or package.
    DependencyRemoved,
    /// Authorization behaviour changed.
    AuthorizationChanged,
    /// A validation step was removed.
    ValidationRemoved,
    /// Database writes changed.
    DatabaseWriteChanged,
    /// Transaction boundaries changed.
    TransactionBoundaryChanged,
    /// An HTTP or exported API contract changed.
    ApiContractChanged,
    /// The declared return type changed.
    ReturnTypeChanged,
}

impl ChangeClass {
    /// Every class in declaration order.
    pub const ALL: [ChangeClass; 16] = [
        Self::ControlFlowChanged,
        Self::ConditionChanged,
        Self::LoopChanged,
        Self::ReturnChanged,
        Self::ExceptionHandlingChanged,
        Self::AsyncBehaviorChanged,
        Self::CallAdded,
        Self::CallRemoved,
        Self::DependencyAdded,
        Self::DependencyRemoved,
        Self::AuthorizationChanged,
        Self::ValidationRemoved,
        Self::DatabaseWriteChanged,
        Self::TransactionBoundaryChanged,
        Self::ApiContractChanged,
        Self::ReturnTypeChanged,
    ];

    /// The wire string (`authorization_changed`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ControlFlowChanged => "control_flow_changed",
            Self::ConditionChanged => "condition_changed",
            Self::LoopChanged => "loop_changed",
            Self::ReturnChanged => "return_changed",
            Self::ExceptionHandlingChanged => "exception_handling_changed",
            Self::AsyncBehaviorChanged => "async_behavior_changed",
            Self::CallAdded => "call_added",
            Self::CallRemoved => "call_removed",
            Self::DependencyAdded => "dependency_added",
            Self::DependencyRemoved => "dependency_removed",
            Self::AuthorizationChanged => "authorization_changed",
            Self::ValidationRemoved => "validation_removed",
            Self::DatabaseWriteChanged => "database_write_changed",
            Self::TransactionBoundaryChanged => "transaction_boundary_changed",
            Self::ApiContractChanged => "api_contract_changed",
            Self::ReturnTypeChanged => "return_type_changed",
        }
    }
}

impl fmt::Display for ChangeClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a classification is exact or relies on heuristics (limited type information).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    /// Derived directly from facts or edges.
    Exact,
    /// Depends on naming conventions or missing type information.
    Heuristic,
}

/// A syntax fact used as evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FactRef {
    /// Which side the fact was read from.
    pub side: DiffSide,
    /// Fact kind.
    pub kind: FactKind,
    /// Position-free key (bounded to 120 characters).
    pub key: String,
    /// Where the fact is on its side.
    pub range: SourceRange,
}

/// One classification of one changed symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClassifiedChange {
    /// The class.
    pub class: ChangeClass,
    /// Facts supporting it, base-side evidence first.
    pub evidence: Vec<FactRef>,
    /// Class-specific detail (subtypes, targets, counts); no source text beyond 120 chars.
    pub detail: BTreeMap<String, serde_json::Value>,
    /// Exact or heuristic.
    pub certainty: Certainty,
}

impl ClassifiedChange {
    /// A change with no evidence or detail yet.
    pub fn new(class: ChangeClass, certainty: Certainty) -> Self {
        Self {
            class,
            evidence: Vec::new(),
            detail: BTreeMap::new(),
            certainty,
        }
    }

    /// Add a detail entry.
    pub fn with(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.detail.insert(key.to_owned(), value.into());
        self
    }
}

/// A symbol touched by a pull request, with both versions and its classes (CHG-001). The
/// DOM-005 shell ([`review_core::change::ChangedSymbol`]) is the projection [`Self::to_core`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedSymbol {
    /// Storage key of `symbol_id`.
    pub symbol_key: SymbolKey,
    /// Canonical id (head id, base id for removed symbols).
    pub symbol_id: SymbolId,
    /// File (head path, base path for removed symbols).
    pub path: RepoPath,
    /// `Head` except for removed symbols.
    pub side: DiffSide,
    /// Range on `side`.
    pub range: SourceRange,
    /// How it changed.
    pub change: ChangeKind,
    /// Base version (absent for added symbols).
    pub base: Option<SymbolVersion>,
    /// Head version (absent for removed symbols).
    pub head: Option<SymbolVersion>,
    /// Comment-only or formatting edit (all hashes equal).
    pub cosmetic: bool,
    /// Indices into the file's hunks.
    pub hunk_ids: Vec<u32>,
    /// Flags.
    pub flags: SymbolFlags,
    /// Change classes (CHG-002..005), sorted by class then evidence order.
    pub classes: Vec<ClassifiedChange>,
}

impl ChangedSymbol {
    /// The DOM-005 shell, `None` for cosmetic records.
    pub fn to_core(&self) -> Option<review_core::change::ChangedSymbol> {
        Some(review_core::change::ChangedSymbol {
            symbol_key: self.symbol_key,
            symbol_id: self.symbol_id.clone(),
            path: self.path.clone(),
            side: self.side,
            range: self.range,
            change: self.change.to_core()?,
        })
    }

    /// Whether the symbol carries `class`.
    pub fn has_class(&self, class: ChangeClass) -> bool {
        self.classes.iter().any(|c| c.class == class)
    }

    /// The distinct classes, sorted.
    pub fn class_set(&self) -> Vec<ChangeClass> {
        let mut out: Vec<ChangeClass> = self.classes.iter().map(|c| c.class).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Bounded copy of a key or expression fragment (at most 120 characters).
pub(crate) fn bounded(text: &str) -> String {
    text.chars().take(120).collect()
}
