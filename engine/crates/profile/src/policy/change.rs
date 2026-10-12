//! What the policy evaluators and pre-checks read about a pull request.
//!
//! This is the slice of the change model (CHG) the evaluators need, expressed over graph keys so
//! the `profile` crate stays independent of `diff-engine`; the pipeline's analysis stage fills it
//! from the `ChangeModel`.

use codegraph::{GraphQuery, NodeKey};

use crate::facts::FactIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Head path (base path for a deletion).
    pub path: String,
    pub status: FileStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolChange {
    Added,
    BodyChanged,
    SignatureChanged,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedSymbol {
    pub key: NodeKey,
    pub id: String,
    pub path: String,
    /// 1-based first line in the head (base for a removal).
    pub line: u32,
    pub change: SymbolChange,
    /// The change alters behavior (CHG classification), not only formatting or comments.
    pub behavioral: bool,
}

/// One line the pull request adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedLine {
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// A changed API endpoint contract (CHG `api_contract_changed`): route, DTO or response type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiContractChange {
    pub endpoint_id: String,
    /// The handler in the head graph.
    pub handler: Option<NodeKey>,
    pub path: String,
    pub line: u32,
    pub what: String,
}

/// The pull request's change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub files: Vec<ChangedFile>,
    pub symbols: Vec<ChangedSymbol>,
    pub added_lines: Vec<AddedLine>,
    pub api_contract_changes: Vec<ApiContractChange>,
}

impl ChangeSet {
    pub fn is_changed_file(&self, path: &str) -> bool {
        self.files.iter().any(|f| f.path == path)
    }

    /// Head paths of files added or modified (not deleted).
    pub fn live_files(&self) -> impl Iterator<Item = &str> {
        self.files
            .iter()
            .filter(|f| f.status != FileStatus::Deleted)
            .map(|f| f.path.as_str())
    }
}

/// The base and head graphs (and their syntax facts) of a pull request.
#[derive(Clone, Copy)]
pub struct GraphPair<'a> {
    pub base: &'a dyn GraphQuery,
    pub head: &'a dyn GraphQuery,
    pub base_facts: &'a FactIndex,
    pub head_facts: &'a FactIndex,
}

impl std::fmt::Debug for GraphPair<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphPair")
            .field("base_nodes", &self.base.node_count())
            .field("head_nodes", &self.head.node_count())
            .finish()
    }
}
