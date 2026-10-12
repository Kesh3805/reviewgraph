//! The narrow change input the impact crate consumes.
//!
//! `impact` is programmed against the DOM-005 entities in `review-core`
//! ([`ChangedSymbol`], [`ChangeClusterKey`](review_core::change::ChangeClusterKey)) plus the
//! handful of change-model facts it actually reads: change classes with their detail tags,
//! added/removed call targets, file dispositions and the CHG-006 aggregates. The CHG-007
//! `PullRequestChangeModel` (diff-engine) is mapped onto a [`ChangeSet`] by a small adapter in
//! the composition root, so the impact graph, clustering and risk engine do not depend on the
//! change model's internal layout.
//!
//! Nothing here is serialized into an output artifact: added-line text (used by the secrets
//! detector) never leaves this crate.

use std::collections::BTreeSet;

use codegraph::{NodeId, NodeKey, NodeKind};
use review_core::change::{ChangedSymbol, FileChangeStatus, SymbolChange};
use review_core::ids::SymbolKey;
use review_core::location::{DiffSide, RepoPath};

/// The 16 PRD §28 change classes (CHG-002), mirrored here so the adapter maps them by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChangeClass {
    ControlFlowChanged,
    ConditionChanged,
    LoopChanged,
    ReturnChanged,
    ExceptionHandlingChanged,
    AsyncBehaviorChanged,
    CallAdded,
    CallRemoved,
    DependencyAdded,
    DependencyRemoved,
    AuthorizationChanged,
    ValidationRemoved,
    DatabaseWriteChanged,
    TransactionBoundaryChanged,
    ApiContractChanged,
    ReturnTypeChanged,
}

impl ChangeClass {
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

    /// The PRD wire name in snake_case, as CHG-002 serializes it.
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

    /// Parses the wire name.
    pub fn from_str_exact(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.as_str() == raw)
    }
}

/// Detail tags a classified change may carry. The adapter derives them from the change model's
/// `detail` maps (subtype strings, flags); the risk rules read only these.
pub mod tags {
    /// `authorization_changed`: an authorization call was removed (CHG-005 rule b).
    pub const AUTHZ_CALL_REMOVED: &str = "authz_call_removed";
    /// `authorization_changed`: a direct role/permission comparison was added (rule c).
    pub const DIRECT_ROLE_COMPARISON_ADDED: &str = "direct_role_comparison_added";
    /// `authorization_changed`: a guard decorator or `AUTHORIZES` edge was removed.
    pub const GUARD_REMOVED: &str = "guard_removed";
    /// `authorization_changed`: a guard decorator or `AUTHORIZES` edge was added.
    pub const GUARD_ADDED: &str = "guard_added";
    /// `database_write_changed`: the write is inside a transaction scope.
    pub const INSIDE_TRANSACTION: &str = "inside_transaction";
    /// `api_contract_changed`: a removed endpoint/field/param or a narrowed type.
    pub const BREAKING: &str = "breaking";
    /// `api_contract_changed`: additive only.
    pub const ADDITIVE: &str = "additive";
    /// `async_behavior_changed`: a floating promise was introduced.
    pub const FLOATING_PROMISE: &str = "floating_promise";
    /// `async_behavior_changed`: an `await` on a database write was removed.
    pub const AWAIT_REMOVED_ON_DB_WRITE: &str = "await_removed_on_db_write";
    /// `exception_handling_changed`: a catch body was emptied.
    pub const SWALLOWED: &str = "swallowed";
}

/// Symbol-level fact tags (TSA-006 facts the risk rules read).
pub mod facts {
    /// A module-level mutable variable is written by the symbol.
    pub const SHARED_STATE_WRITE: &str = "shared_state_write";
    /// `Promise.all` over database writes.
    pub const PROMISE_ALL_DB_WRITES: &str = "promise_all_db_writes";
    /// Only the literal initializer of a constant/variable changed.
    pub const LITERAL_INITIALIZER_ONLY: &str = "literal_initializer_only";
    /// A filesystem call takes a request-derived path argument.
    pub const REQUEST_DERIVED_PATH: &str = "request_derived_path";
    /// `JSON.parse` on external input was added.
    pub const JSON_PARSE_EXTERNAL: &str = "json_parse_external";
}

/// One classified change of a symbol.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassInput {
    pub class: ChangeClass,
    pub tags: BTreeSet<String>,
    /// Classification confidence in `[0, 1]` (`Exact` = 1.0, `Heuristic` lower).
    pub confidence: f32,
}

impl ClassInput {
    pub fn new(class: ChangeClass) -> Self {
        Self {
            class,
            tags: BTreeSet::new(),
            confidence: 1.0,
        }
    }

    pub fn tag(mut self, tag: &str) -> Self {
        self.tags.insert(tag.to_owned());
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn has(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }
}

/// An added or removed call (CHG-003 `BoundCall`, reduced). Removed calls are resolved on base,
/// added calls on head.
#[derive(Debug, Clone, PartialEq)]
pub struct CallInput {
    pub target: Option<NodeKey>,
    pub target_id: Option<String>,
    /// Normalized callee text, e.g. `this.permissionService.check`.
    pub callee_text: String,
    pub confidence: f32,
}

impl CallInput {
    /// A call bound to the node with canonical id `target_id`.
    pub fn to(target_id: &str, callee_text: &str) -> Self {
        Self {
            target: Some(NodeId::from_canonical(target_id).key()),
            target_id: Some(target_id.to_owned()),
            callee_text: callee_text.to_owned(),
            confidence: 1.0,
        }
    }

    /// An unresolved call, known by its text only.
    pub fn unresolved(callee_text: &str) -> Self {
        Self {
            target: None,
            target_id: None,
            callee_text: callee_text.to_owned(),
            confidence: 1.0,
        }
    }

    /// The target key, from `target` or derived from `target_id`.
    pub fn target_key(&self) -> Option<NodeKey> {
        self.target.or_else(|| {
            self.target_id
                .as_ref()
                .map(|id| NodeId::from_canonical(id.as_str()).key())
        })
    }
}

/// A changed symbol as the impact crate sees it: the DOM-005 entity plus CHG-001..005 facts.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInput {
    pub symbol: ChangedSymbol,
    /// Graph node kind, when the adapter knows it (the graph is consulted otherwise).
    pub kind: Option<NodeKind>,
    /// Formatting/comment-only change (CHG-001).
    pub cosmetic: bool,
    /// Lives in a generated or vendored file.
    pub generated: bool,
    /// Exported or otherwise public API (`PUBLIC_API`).
    pub exported: bool,
    /// Lives in a test file.
    pub test: bool,
    /// The symbol's file did not parse cleanly; facts may be missing.
    pub parse_degraded: bool,
    /// For renames: the body changed as well.
    pub rename_body_changed: bool,
    pub classes: Vec<ClassInput>,
    pub added_calls: Vec<CallInput>,
    pub removed_calls: Vec<CallInput>,
    /// Fact tags, see [`facts`].
    pub facts: BTreeSet<String>,
    /// Changed lines attributed to the symbol (cluster size).
    pub size_lines: u32,
}

impl SymbolInput {
    /// Wraps a DOM-005 entity with no classes and no flags.
    pub fn new(symbol: ChangedSymbol) -> Self {
        let size_lines = symbol
            .range
            .end
            .line
            .saturating_sub(symbol.range.start.line)
            .saturating_add(1);
        Self {
            symbol,
            kind: None,
            cosmetic: false,
            generated: false,
            exported: false,
            test: false,
            parse_degraded: false,
            rename_body_changed: false,
            classes: Vec::new(),
            added_calls: Vec::new(),
            removed_calls: Vec::new(),
            facts: BTreeSet::new(),
            size_lines,
        }
    }

    pub fn key(&self) -> SymbolKey {
        self.symbol.symbol_key
    }

    pub fn id(&self) -> &str {
        self.symbol.symbol_id.as_str()
    }

    pub fn path(&self) -> &RepoPath {
        &self.symbol.path
    }

    /// Removed symbols are resolved on the base graph.
    pub fn is_removed(&self) -> bool {
        matches!(self.symbol.change, SymbolChange::Removed) || self.symbol.side == DiffSide::Base
    }

    pub fn has_class(&self, class: ChangeClass) -> bool {
        self.classes.iter().any(|c| c.class == class)
    }

    /// Cosmetic or generated: listed, never expanded.
    pub fn is_skipped(&self) -> bool {
        self.cosmetic || self.generated
    }
}

/// How the diff engine disposed of a file (DIFF-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum FileDisposition {
    #[default]
    Analyzed,
    Generated,
    Vendored,
    Binary,
    Large,
    Minified,
}

/// One added line of a text file (new side), for the secrets detector.
#[derive(Clone, PartialEq, Eq)]
pub struct AddedLine {
    pub line: u32,
    pub text: String,
}

impl std::fmt::Debug for AddedLine {
    /// Never prints the text: added lines may hold secrets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddedLine")
            .field("line", &self.line)
            .field("len", &self.text.len())
            .finish()
    }
}

/// An import-only change of a file (RISK-006 import sorting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportChange {
    /// The multiset of import specifiers and bindings is unchanged.
    pub bindings_equal: bool,
    /// The order of side-effect imports (no bindings) changed.
    pub side_effect_order_changed: bool,
}

/// A changed file.
#[derive(Debug, Clone, PartialEq)]
pub struct FileInput {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub status: FileChangeStatus,
    pub disposition: FileDisposition,
    /// Every hunk is whitespace- or comment-only.
    pub cosmetic_only: bool,
    /// For renames and copies.
    pub rename_similarity: Option<f32>,
    /// Only import lines changed.
    pub import_only: Option<ImportChange>,
    pub parse_degraded: bool,
    /// Added lines of a text file; `None` when the head blob could not be read.
    pub added_lines: Option<Vec<AddedLine>>,
    pub additions: u32,
    pub deletions: u32,
}

impl FileInput {
    pub fn new(path: RepoPath, status: FileChangeStatus) -> Self {
        Self {
            path,
            old_path: None,
            status,
            disposition: FileDisposition::Analyzed,
            cosmetic_only: false,
            rename_similarity: None,
            import_only: None,
            parse_degraded: false,
            added_lines: Some(Vec::new()),
            additions: 0,
            deletions: 0,
        }
    }

    /// Generated, vendored, binary, large or minified files are not analyzed.
    pub fn is_unanalyzed(&self) -> bool {
        self.disposition != FileDisposition::Analyzed
    }
}

/// A changed HTTP API (CHG-006 `ChangedAPI`, reduced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiInput {
    /// Endpoint node id, e.g. `http:PUT /users/{}`.
    pub endpoint: String,
    pub handler: Option<SymbolKey>,
    /// DTOs, guards and other changed symbols belonging to the endpoint.
    pub related: Vec<SymbolKey>,
    pub breaking: bool,
    pub auth_changed: bool,
}

/// How a dependency version moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BumpKind {
    Patch,
    Minor,
    Major,
    Prerelease,
    Downgrade,
}

/// What happened to a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyChange {
    Added,
    Removed,
    Bumped(BumpKind),
    SourceChanged,
}

/// A changed dependency (CHG-006 `ChangedDependency`, reduced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyInput {
    pub name: String,
    /// The manifest (or, for lockfile-only changes, the lockfile) that changed.
    pub manifest: RepoPath,
    pub change: DependencyChange,
    pub lockfile_only: bool,
}

/// Migration or entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaKind {
    Migration,
    Entity,
}

/// A changed schema artifact (CHG-006 `ChangedSchema`, reduced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaInput {
    pub kind: SchemaKind,
    pub path: RepoPath,
    pub destructive: bool,
    /// An already-applied migration file was edited.
    pub modified_existing: bool,
    pub table: Option<String>,
}

/// A changed test file (CHG-006 `ChangedTest`, reduced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestChangeInput {
    pub path: RepoPath,
    pub targets: Vec<SymbolKey>,
}

/// A test that mocks a class (`jest.mock(path)`, `useValue` provider override), from NEST-006
/// facts. `test` is the test case, suite or test-file node; `target` the mocked class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MockFact {
    pub test: NodeKey,
    pub target: NodeKey,
}

/// Everything the impact crate reads about a pull request's changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeSet {
    /// The change model's own `input_hash`.
    pub input_hash: String,
    pub head_snapshot: String,
    pub base_snapshot: String,
    pub files: Vec<FileInput>,
    pub symbols: Vec<SymbolInput>,
    pub apis: Vec<ApiInput>,
    pub dependencies: Vec<DependencyInput>,
    pub schemas: Vec<SchemaInput>,
    pub tests: Vec<TestChangeInput>,
    pub mocks: Vec<MockFact>,
}

impl ChangeSet {
    pub fn symbol(&self, key: SymbolKey) -> Option<&SymbolInput> {
        self.symbols.iter().find(|symbol| symbol.key() == key)
    }

    pub fn file(&self, path: &RepoPath) -> Option<&FileInput> {
        self.files.iter().find(|file| &file.path == path)
    }

    /// Keys of every changed symbol, sorted.
    pub fn symbol_keys(&self) -> BTreeSet<SymbolKey> {
        self.symbols.iter().map(SymbolInput::key).collect()
    }
}
