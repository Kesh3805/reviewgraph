//! References the linker could not turn into an edge (CG-004 storage, CG-005 production).
//!
//! Unresolved references are first-class graph state, not a log line: an `External` reason is
//! what later produces a `pkg:` node, an `Ambiguous` reason is what the name-index delta
//! (INC-004) re-checks when a new file appears, and the verification stage asserts that an
//! incremental update does not invent or destroy one silently.

use std::fmt;

use analysis_ir::reference::RefKind;
use review_core::location::RepoPath;
use serde::{Deserialize, Serialize};

use crate::edge::Location;
use crate::node_id::NodeKey;

/// Why a reference stayed unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum UnresolvedReason {
    /// The name lives outside the repository (an npm package, a global).
    External = 0,
    /// No candidate matched after the cascade ran.
    NotFound = 1,
    /// Several candidates matched and the fan-out limit stopped the cascade.
    Ambiguous = 2,
    /// The module resolver reported an error for the specifier.
    ResolverError = 3,
    /// A re-export cycle was cut rather than followed forever.
    ReexportCycle = 4,
    /// `LinkConfig::max_reexport_depth` was reached first.
    DepthExceeded = 5,
}

impl UnresolvedReason {
    pub const ALL: [UnresolvedReason; 6] = [
        Self::External,
        Self::NotFound,
        Self::Ambiguous,
        Self::ResolverError,
        Self::ReexportCycle,
        Self::DepthExceeded,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::External => "EXTERNAL",
            Self::NotFound => "NOT_FOUND",
            Self::Ambiguous => "AMBIGUOUS",
            Self::ResolverError => "RESOLVER_ERROR",
            Self::ReexportCycle => "REEXPORT_CYCLE",
            Self::DepthExceeded => "DEPTH_EXCEEDED",
        }
    }

    pub fn from_str_exact(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(raw: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_u8() == raw)
    }
}

impl fmt::Display for UnresolvedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for UnresolvedReason {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s).ok_or_else(|| format!("UnresolvedReason has no member {s:?}"))
    }
}

/// One reference that did not become an edge.
///
/// `file` + `ordinal` identify the reference inside its file, so a re-link of that file can
/// replace exactly this row (INC-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedRef {
    /// File the reference was written in.
    pub file: RepoPath,
    /// Position of the reference in the file's reference list, 0-based.
    pub ordinal: u32,
    /// Symbol the reference was written in, when known.
    pub from: Option<NodeKey>,
    /// The name that could not be resolved, as written (already normalized by the analyzer).
    pub name: String,
    pub kind: RefKind,
    /// Import specifier the name came from, for `External`/`ResolverError`.
    pub import_specifier: Option<String>,
    pub location: Location,
    pub reason: UnresolvedReason,
    /// How many candidates the cascade had before it stopped.
    pub candidate_count: u16,
}

impl UnresolvedRef {
    /// Total heap footprint, for `Graph::heap_size_bytes`.
    pub fn heap_size_bytes(&self) -> usize {
        size_of::<UnresolvedRef>()
            + self.file.as_str().len()
            + self.name.len()
            + self.import_specifier.as_ref().map(String::len).unwrap_or(0)
    }
}
