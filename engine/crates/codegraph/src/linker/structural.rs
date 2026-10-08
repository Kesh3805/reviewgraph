//! Structural edges: containment, declarations, exports and file-level imports (CG-005).
//!
//! These are the edges that hold regardless of what any reference resolves to, so they are
//! derived entirely from the [`SymbolTable`] and the resolver and carry
//! [`ResolvedBy::Structural`] with [`Provenance::Analyzer`]. Because they are a pure function of
//! the table, an incremental rebuild produces exactly the same structural edges as a full one.
//!
//! * `Repository CONTAINS Directory`, `Directory CONTAINS Directory|File`,
//! * `File DECLARES`/`CONTAINS` every top-level symbol it declares,
//! * `Class CONTAINS` its members,
//! * `File EXPORTS` its exported symbols,
//! * `File IMPORTS File` for every import that resolved to a file inside the repository.

use std::collections::BTreeMap;
use std::sync::Arc;

use analysis_ir::traits::{ModuleResolver, Resolution, ResolveKind};
use analysis_ir::unit::ParsedUnit;

use crate::confidence;
use crate::edge::{Edge, Provenance, ResolvedBy};
use crate::edge_kind::EdgeKind;
use crate::graph::NodeInput;
use crate::linker::symbol_table::{ExportTarget, FileSymbols, SymbolTable};
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;

/// Structural nodes a whole snapshot needs: the repository singleton and every directory that
/// contains an analyzed file.
#[must_use]
pub fn repository_node(name: &str) -> NodeInput {
    NodeInput::new(NodeId::repository(), NodeKind::Repository, name).qualified_name(name.to_owned())
}

/// The repository node plus one node per directory, from outermost to innermost, plus the
/// `CONTAINS` edges between them.
///
/// The chain is built per file by [`directory_nodes_for`] and merged here so two files in the
/// same directory share one directory node.
#[derive(Debug, Default)]
pub struct RepositoryStructure {
    pub nodes: Vec<NodeInput>,
    pub edges: Vec<Edge>,
}

impl RepositoryStructure {
    /// Folds one file's directory chain into the structure.
    pub fn add_file(&mut self, path: &review_core::location::RepoPath) {
        let chain = NodeId::ancestor_directories(path);
        let mut parent = NodeId::repository().key();
        for directory in chain {
            let key = directory.key();
            if self.nodes.iter().all(|node| node.id != directory) {
                let name = directory
                    .as_str()
                    .strip_prefix("dir:")
                    .unwrap_or(directory.as_str())
                    .to_owned();
                self.nodes.push(
                    NodeInput::new(directory.clone(), NodeKind::Directory, name.clone())
                        .qualified_name(name),
                );
            }
            self.edges.push(contains(parent, key, path));
            parent = key;
        }
        self.edges
            .push(contains(parent, NodeId::file(path).key(), path));
    }
}

fn contains(parent: NodeKey, child: NodeKey, path: &review_core::location::RepoPath) -> Edge {
    Edge::new(
        EdgeKind::Contains,
        parent,
        child,
        confidence::confidence_of(ResolvedBy::Structural),
        ResolvedBy::Structural,
        Provenance::Analyzer,
    )
    .with_origin_file(path.clone())
}

/// The structural edges one file owns: its declarations, exports and imports.
///
/// `None` location: a containment edge is not observed at one position, and giving it one would
/// make per-file replacement ambiguous. The `origin_file` is still set, so the edge is owned by
/// the file and INC-005 replaces it with the rest of that file's contribution.
#[must_use]
pub fn file_structural_edges(
    unit: &ParsedUnit,
    file: &FileSymbols,
    resolver: &dyn ModuleResolver,
) -> Vec<Edge> {
    let mut out = Vec::new();
    let file_key = file.file_key;
    let mut declared = 0u32;

    for symbol in file.symbols.iter().skip(1) {
        match symbol.parent {
            None => {
                // `DECLARES` for a file-level symbol, `CONTAINS` as well so a containment walk
                // from the repository reaches it without knowing about `DECLARES`.
                out.push(Edge::new(
                    EdgeKind::Declares,
                    file_key,
                    symbol.key,
                    confidence::confidence_of(ResolvedBy::Structural),
                    ResolvedBy::Structural,
                    Provenance::Analyzer,
                ));
                out.push(Edge::new(
                    EdgeKind::Contains,
                    file_key,
                    symbol.key,
                    confidence::confidence_of(ResolvedBy::Structural),
                    ResolvedBy::Structural,
                    Provenance::Analyzer,
                ));
                declared += 1;
            }
            Some(parent) => {
                out.push(Edge::new(
                    EdgeKind::Contains,
                    parent,
                    symbol.key,
                    confidence::confidence_of(ResolvedBy::Structural),
                    ResolvedBy::Structural,
                    Provenance::Analyzer,
                ));
            }
        }
        let _ = declared;
    }

    for target in file.exports.values() {
        if let ExportTarget::Local(key) = target {
            out.push(Edge::new(
                EdgeKind::Exports,
                file_key,
                *key,
                confidence::confidence_of(ResolvedBy::Structural),
                ResolvedBy::Structural,
                Provenance::Analyzer,
            ));
        }
    }

    // One `IMPORTS` edge per distinct resolved in-repository specifier, in specifier order so
    // the output does not depend on reference order.
    let mut targets: BTreeMap<String, NodeKey> = BTreeMap::new();
    for import in &unit.imports {
        if !targets.contains_key(&import.specifier) {
            let Resolution::File { path, .. } =
                resolver.resolve(&unit.file, &import.specifier, ResolveKind::Import)
            else {
                continue;
            };
            targets.insert(import.specifier.clone(), NodeId::file(&path).key());
        }
    }
    for target in targets.values() {
        out.push(Edge::new(
            EdgeKind::Imports,
            file_key,
            *target,
            confidence::confidence_of(ResolvedBy::Structural),
            ResolvedBy::Structural,
            Provenance::Analyzer,
        ));
    }

    // Every structural edge is owned by the file whose symbols it describes, which is what lets
    // an incremental update replace one file's contribution without touching the rest.
    out = out
        .into_iter()
        .map(|edge| edge.with_origin_file(unit.file.clone()))
        .collect();
    out
}

/// Every structural edge of a snapshot: the repository/directory chain plus each file's own
/// contribution.
#[must_use]
pub fn structural_edges(
    units: &[Arc<ParsedUnit>],
    table: &SymbolTable,
    resolver: &dyn ModuleResolver,
) -> Vec<Edge> {
    let mut out = Vec::new();
    for unit in units {
        if let Some(file) = table.get(&unit.file) {
            out.extend(file_structural_edges(unit, file, resolver));
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::graph::FileIx;
    use review_core::location::RepoPath;

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).unwrap()
    }

    #[test]
    fn directory_chain_is_shared_and_nested() {
        let mut structure = RepositoryStructure::default();
        structure.add_file(&path("src/users/users.service.ts"));
        structure.add_file(&path("src/users/users.controller.ts"));
        structure.add_file(&path("src/app.ts"));

        let directories: Vec<&str> = structure
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        assert_eq!(directories, vec!["dir:.", "dir:src", "dir:src/users"]);

        let repo = NodeId::repository().key();
        let root = NodeId::root_directory().key();
        let src = NodeId::from_canonical("dir:src".to_owned()).key();
        let users = NodeId::from_canonical("dir:src/users".to_owned()).key();
        let contains = |from: NodeKey, to: NodeKey| {
            structure
                .edges
                .iter()
                .any(|e| e.source == from && e.target == to && e.kind == EdgeKind::Contains)
        };
        assert!(contains(repo, root));
        assert!(contains(root, src));
        assert!(contains(src, users));
        assert!(contains(
            users,
            NodeId::file(&path("src/users/users.service.ts")).key()
        ));
        assert!(contains(
            users,
            NodeId::file(&path("src/users/users.controller.ts")).key()
        ));
        assert!(contains(src, NodeId::file(&path("src/app.ts")).key()));
        assert!(
            !contains(root, NodeId::file(&path("src/app.ts")).key()),
            "a file is contained by its own directory, not by every ancestor"
        );
    }

    #[test]
    fn structural_edges_are_structural_and_owned_by_their_file() {
        let path = path("src/a.ts");
        let mut builder = crate::graph::GraphBuilder::new(crate::schema::SCHEMA_VERSION);
        builder
            .add_file(crate::graph::FileInput::placeholder(path.clone()))
            .unwrap();
        assert_eq!(builder.file_count(), 1);
        assert_eq!(FileIx::new(0).get(), 0);
        let repository = repository_node("reviewgraph");
        assert_eq!(repository.id, NodeId::repository());
        assert_eq!(repository.kind, NodeKind::Repository);
        assert!(repository.file.is_none());
    }
}
