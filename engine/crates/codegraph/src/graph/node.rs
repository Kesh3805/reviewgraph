//! Nodes and their attribute block (CG-004).

use std::fmt;

use analysis_ir::symbol::Visibility;
use review_core::location::SourceRange;
use review_core::symbol::Hash128;
use serde::{Deserialize, Serialize};

use crate::node_id::NodeKey;
use crate::node_kind::NodeKind;

use super::file::FileIx;
use super::interner::StrId;

/// Per-node boolean attributes, as a bit set so the node table stays small (CG-004).
///
/// `repr(u8)` with fixed bits: the values are persisted by `graph-storage` and read back by the
/// verifier, so reordering them is a schema change. Serialized as its raw `u8` rather than as a
/// list of flag names, which keeps the wire form identical to the stored column.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[repr(transparent)]
pub struct NodeFlags(u8);

impl NodeFlags {
    /// Declared by an `export`/`pub`/`@api` statement.
    pub const EXPORTED: Self = Self(1);
    /// Produced by a code generator; never hand-written.
    pub const GENERATED: Self = Self(2);
    /// Lives under a test path or in a `*.test.*` file.
    pub const TEST: Self = Self(4);
    /// `abstract`, `interface`-style or `@abstract`.
    pub const ABSTRACT: Self = Self(8);
    /// `static` member.
    pub const STATIC: Self = Self(16);
    /// `async` callable.
    pub const ASYNC: Self = Self(32);
    /// An enum member, which is stored as a `Constant` node with this bit set.
    pub const ENUM_MEMBER: Self = Self(64);

    pub const EMPTY: Self = Self(0);
    /// All seven bits.
    pub const ALL_BITS: u8 = 127;

    /// Keeps only the defined bits, so an unknown bit from a newer writer is dropped rather
    /// than round-tripped into a value nothing understands.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL_BITS)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Every set flag, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        const NAMED: [(NodeFlags, &str); 7] = [
            (NodeFlags::EXPORTED, "EXPORTED"),
            (NodeFlags::GENERATED, "GENERATED"),
            (NodeFlags::TEST, "TEST"),
            (NodeFlags::ABSTRACT, "ABSTRACT"),
            (NodeFlags::STATIC, "STATIC"),
            (NodeFlags::ASYNC, "ASYNC"),
            (NodeFlags::ENUM_MEMBER, "ENUM_MEMBER"),
        ];
        NAMED
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }
}

impl std::ops::BitOr for NodeFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for NodeFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl fmt::Display for NodeFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.names();
        if names.is_empty() {
            return f.write_str("EMPTY");
        }
        write!(f, "{}", names.join("|"))
    }
}

/// Everything about a node that is not identity, kind or position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAttrs {
    pub visibility: Visibility,
    pub flags: NodeFlags,
    /// Display signature, already truncated by the analyzer (at most 512 characters).
    pub signature: Option<StrId>,
    pub body_hash: Option<Hash128>,
    pub signature_hash: Option<Hash128>,
    /// Enclosing symbol, for members that are stored flat in the node table.
    pub parent: Option<NodeKey>,
    /// Analyzer-specific key/value pairs, sorted by key by the builder so the graph stays
    /// byte-identical whatever order the inputs arrived in.
    pub extra: Option<Box<[(StrId, StrId)]>>,
}

impl Default for NodeAttrs {
    fn default() -> Self {
        Self {
            visibility: Visibility::Public,
            flags: NodeFlags::EMPTY,
            signature: None,
            body_hash: None,
            signature_hash: None,
            parent: None,
            extra: None,
        }
    }
}

/// One row of the node table.
///
/// `id`, `name`, `qualified_name` and `attrs.signature` are [`StrId`]s into the graph's
/// interner, which is what keeps a million-node table at roughly 56 bytes a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeData {
    /// blake3-128 of the canonical [`crate::node_id::NodeId`].
    pub key: NodeKey,
    pub kind: NodeKind,
    /// Canonical node id, for example `ts:src/a.ts#B/c/function`.
    pub id: StrId,
    pub name: StrId,
    pub qualified_name: StrId,
    /// Owning file; `None` for synthetic nodes (repository, queue, env var, package).
    pub file: Option<FileIx>,
    pub range: Option<SourceRange>,
    pub attrs: NodeAttrs,
}

impl NodeData {
    /// Rough heap footprint of one node, used by the cache sizing math.
    pub fn heap_size_bytes(&self) -> usize {
        let extra = self
            .attrs
            .extra
            .as_ref()
            .map(|pairs| pairs.len() * (size_of::<StrId>() * 2 + 16))
            .unwrap_or(0);
        size_of::<NodeData>() + extra
    }
}
