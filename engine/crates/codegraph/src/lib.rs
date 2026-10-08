//! An in-memory, versioned property graph: the shared representation every review stage reads
//! (target-architecture §2, §3.3).
//!
//! The crate is organised as a set of small, independently testable layers:
//!
//! * [`node_kind`] / [`node_id`] — the node taxonomy of PRD §17 and the deterministic
//!   synthetic ID schemes of target-architecture §3.3 (CG-001).
//! * [`edge_kind`] / [`edge`] — the 33 stored edge kinds, the reverse views that are never
//!   stored, and the typed edge payload with its confidence (CG-002).
//! * [`confidence`] — the single `resolved_by → confidence` table (CG-003).
//! * [`graph`] — the builder, the interned node/edge/file tables and the kind-partitioned CSR
//!   adjacency (CG-004).
//! * [`linker`] — the resolution cascade that turns IR references into typed edges (CG-005).
//! * [`framework`] — framework facts mapped onto generic nodes and edges (CG-006).
//! * [`query`] — the read interface every consumer uses, plus bounded BFS, shortest paths and
//!   subgraphs (CG-007/008/009).
//! * [`delta`] / [`overlay`] — the in-memory change representation and the PR-head view over a
//!   base graph (CG-010).
//! * [`codec`] — the versioned wire format (CG-011).
//! * [`validate`] / [`compare`] — the consistency oracle (CG-012).
//! * [`schema_rules`] — the endpoint-kind matrix the validator warns about (CG-002).
//! * [`schema`] — the one schema version every reader and writer agrees on.
//!
//! Traversals always take an explicit budget and report truncation; nothing in this crate
//! panics, and every public type derives `Debug`.

pub mod codec;
pub mod compare;
pub mod confidence;
pub mod delta;
pub mod edge;
pub mod edge_kind;
pub mod error;
pub mod framework;
pub mod graph;
pub mod linker;
pub mod node_id;
pub mod node_kind;
pub mod overlay;
pub mod query;
pub mod schema;
pub mod schema_rules;
pub mod validate;

#[cfg(feature = "testkit")]
pub mod testkit;

mod schema_util;

pub use codec::{
    decode_delta, decode_graph, encode_delta, encode_graph, CodecError, DecodeLimits, EncodeStats,
    GraphWire, Header, HEADER_LEN, MAGIC,
};
pub use compare::{compare, CompareOptions, GraphDiffReport};
pub use confidence::{confidence_of, derived, Confidence, ConfidenceError, LINKER_VERSION, TABLE};
pub use delta::{FileChange, FileChangeKind, GraphDelta, LineageRecord, LineageTransition};
pub use edge::{Edge, EdgeFlags, EdgeIdentity, Location, Provenance, ResolvedBy};
pub use edge_kind::{
    Direction, EdgeKind, EdgeKindSet, EdgeSelector, ReverseView, UnknownEdgeKind, ALL_EDGE_KINDS,
};
pub use error::{Error, Result};
pub use framework::{
    FactCategory, FactIssue, FactIssueCode, FrameworkMapper, FrameworkOutput, GlobalFact,
    SyntheticNode,
};
pub use graph::{
    Csr, EdgeData, EdgeIx, FileEntry, FileInput, FileIx, Graph, GraphBuildError, GraphBuilder,
    GraphStats, Interner, NodeAttrs, NodeData, NodeFlags, NodeInput, NodeInputAttrs, NodeIx, StrId,
    UnresolvedReason, UnresolvedRef,
};
pub use linker::{
    FileLinkResult, LinkConfig, LinkInput, LinkOutput, LinkStats, Linker, NameIndex, NameLookup,
    ResolutionDeps, SymbolTable,
};
pub use node_id::{
    is_env_name, normalize_db_ident, normalize_db_schema, normalize_http_method,
    normalize_http_path, normalize_package_spec, normalize_test_component, NodeId, NodeIdError,
    NodeKey, RESERVED_PREFIXES,
};
pub use node_kind::{NodeCategory, NodeKind, ALL_NODE_KINDS};
pub use overlay::{validate_delta_local, GraphOverlay, OverlayError};
pub use query::{
    bounded_bfs, clamp_max_nodes, shortest_path, subgraph, EdgeFilter, EdgeRef, EdgeStep, FileView,
    GraphPath, GraphQuery, GraphQueryExt, NodeRef, PathError, PathResult, PathSpec, Subgraph,
    SubgraphEdge, SubgraphNode, SubgraphSpec, TraversalError, TraversalResult, TraversalSpec,
    Truncation, Visit, SUBGRAPH_MAX_NODES,
};
pub use schema::SCHEMA_VERSION;
pub use validate::{validate, IssueCode, Severity, ValidationIssue, ValidationReport};
