//! The linker: IR references become typed graph edges (CG-005).
//!
//! # Shape
//!
//! ```text
//! Linker::build_tables(units) -> (SymbolTable, NameIndex)
//! Linker::link_file(unit, &tables, &names, &resolver, &config) -> FileLinkResult
//! Linker::link_all(LinkInput) -> LinkOutput
//! ```
//!
//! `link_all` is `link_file` over every file in parallel plus the repository/directory layer,
//! and that equivalence is asserted by a test: an incremental rebuild re-runs `link_file` for
//! the files it touched and must land on exactly the same edges (INC-005).
//!
//! # Determinism
//!
//! Resolution is a pure function of `(units, resolver behaviour, LinkConfig, LINKER_VERSION)`.
//! `link_all` collects from an indexed parallel iterator, so results stay in path order no
//! matter how many rayon threads ran it, and no `HashMap` is ever iterated to produce output.
//!
//! # Invariants
//!
//! * No file I/O and no panics: a resolver error becomes an unresolved reference on the affected
//!   references only, never a failure of the whole link.
//! * The resolver must not resolve outside the repository root (TSA-009 contract); the linker
//!   additionally refuses any target file that is not in the [`SymbolTable`].

pub mod name_index;
pub mod overrides;
pub mod resolve;
pub mod structural;
pub mod symbol_table;

pub use name_index::{NameIndex, NameIndexOverlay, NameLookup};
pub use resolve::{ResolveContext, ResolveOutput};
pub use structural::RepositoryStructure;
pub use symbol_table::{
    file_symbols, symbol_node_id, ExportTarget, FileSymbols, ImportBindingInfo, SymbolInfo,
    SymbolTable, TypeRef,
};

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use analysis_ir::traits::ModuleResolver;
use analysis_ir::unit::ParsedUnit;
use rayon::prelude::*;
use review_core::location::RepoPath;

use crate::edge::{Edge, Provenance, ResolvedBy};
use crate::edge_kind::EdgeKind;
use crate::framework::{FrameworkMapper, GlobalFact};
use crate::graph::{NodeInput, UnresolvedRef};
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;

/// What this file's resolution consulted (CG-005), so an incremental update can tell whether a
/// re-link is still valid after some other file changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolutionDeps {
    /// Every name handed to the name index, sorted.
    pub names: BTreeSet<String>,
    /// Every file whose export table was consulted, sorted.
    pub files: BTreeSet<RepoPath>,
}

/// Linker knobs. These are part of the linker's identity: a different value can produce a
/// different graph, so they belong in the recorded analyzer versions alongside
/// [`crate::LINKER_VERSION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    /// Most candidates an ambiguous name may fan out to before it is reported unresolved.
    pub max_ambiguous_fanout: u8,
    /// How far a `ReExport`/`StarFrom` chain is followed before it is cut.
    pub max_reexport_depth: u8,
    /// How many superclass levels a member lookup walks.
    pub max_super_depth: u8,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            max_ambiguous_fanout: 3,
            max_reexport_depth: 8,
            max_super_depth: 8,
        }
    }
}

/// Everything `link_all` needs.
///
/// `Debug` prints the resolver and the name index by name only: both are trait objects whose
/// implementations live in other crates.
pub struct LinkInput<'a> {
    pub units: &'a [Arc<ParsedUnit>],
    pub resolver: &'a dyn ModuleResolver,
    pub config: &'a LinkConfig,
    /// Name of the repository node, for the `repo:/` singleton.
    pub repository_name: &'a str,
}

impl fmt::Debug for LinkInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkInput")
            .field("units", &self.units.len())
            .field("config", &self.config)
            .field("repository_name", &self.repository_name)
            .finish_non_exhaustive()
    }
}

/// One file's contribution.
#[derive(Debug)]
pub struct FileLinkResult {
    pub path: RepoPath,
    /// Structural nodes: the file node and its declared symbols.
    pub nodes: Vec<NodeInput>,
    /// Synthetic package nodes this file's imports created (`pkg:npm/lodash`).
    pub synthetic_nodes: Vec<NodeInput>,
    /// Structural + reference + framework edges, in a deterministic order.
    pub edges: Vec<Edge>,
    pub unresolved: Vec<UnresolvedRef>,
    pub deps: ResolutionDeps,
    /// Facts that only make sense once every file is mapped (APP_GUARD-style guards, route
    /// prefixes).
    pub global_facts: Vec<GlobalFact>,
    /// Diagnostics for facts whose attribute contract was not satisfied (CG-006).
    pub fact_issues: Vec<crate::framework::FactIssue>,
    /// Kind refinements the framework mapper asked for, applied by `link_all`.
    pub refinements: Vec<(NodeKey, NodeKind)>,
}

impl FileLinkResult {
    /// An empty contribution for `path`.
    #[must_use]
    pub fn empty(path: RepoPath) -> Self {
        Self {
            path,
            nodes: Vec::new(),
            synthetic_nodes: Vec::new(),
            edges: Vec::new(),
            unresolved: Vec::new(),
            deps: ResolutionDeps::default(),
            global_facts: Vec::new(),
            fact_issues: Vec::new(),
            refinements: Vec::new(),
        }
    }
}

/// Everything a full link produced.
#[derive(Debug, Default)]
pub struct LinkOutput {
    pub nodes: Vec<NodeInput>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<UnresolvedRef>,
    pub global_facts: Vec<GlobalFact>,
    pub fact_issues: Vec<crate::framework::FactIssue>,
    pub stats: LinkStats,
}

/// Counters for the `graph.link` span.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub files: usize,
    pub symbols: usize,
    pub references: usize,
    pub edges: usize,
    pub unresolved: usize,
    pub synthetic_nodes: usize,
}

/// The linker itself: a namespace of pure functions over the tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Linker;

impl Linker {
    /// Per-file symbol tables and the repository name index, in `O(S)`.
    #[must_use]
    pub fn build_tables(units: &[Arc<ParsedUnit>]) -> (SymbolTable, NameIndex) {
        let table = SymbolTable::build(units);
        let names = NameIndex::build(&table);
        (table, names)
    }

    /// Links one file. This is the entry point INC-005 re-runs for a changed file and INC-006
    /// for an unchanged one whose dependents moved.
    ///
    /// Reads nothing but the tables, the name index and the resolver, so its result depends only
    /// on that file plus what those shared structures say.
    #[must_use]
    pub fn link_file(
        unit: &ParsedUnit,
        tables: &SymbolTable,
        names: &dyn NameLookup,
        resolver: &dyn ModuleResolver,
        config: &LinkConfig,
    ) -> FileLinkResult {
        let Some(file) = tables.get(&unit.file) else {
            // A unit with no table entry cannot be linked; the caller built the table from the
            // same units, so this is unreachable in practice and reported as "no output".
            return FileLinkResult::empty(unit.file.clone());
        };

        let resolution = ResolveContext::new(unit, file, tables, names, resolver, config).run();
        let structural = structural::file_structural_edges(unit, file, resolver);

        // Framework facts are mapped after reference resolution, because `TESTS` edges and the
        // database edges are derived from the resolved call edges of this file (CG-006).
        let framework = FrameworkMapper::map_file(&unit.framework, file, &resolution.edges);

        // The ecosystem comes from the resolution itself, so a Cargo dependency and an npm one
        // get their own package nodes without the linker hard-coding an ecosystem.
        let mut synthetic_nodes: BTreeMap<NodeKey, NodeInput> = BTreeMap::new();
        for (ecosystem, name) in &resolution.packages {
            let Ok(id) = NodeId::package(ecosystem, name) else {
                continue;
            };
            let qualified = format!("{ecosystem}/{name}");
            synthetic_nodes.entry(id.key()).or_insert_with(|| {
                NodeInput::new(id, NodeKind::ExternalDependency, name.clone())
                    .qualified_name(qualified)
            });
        }
        for node in &framework.nodes {
            synthetic_nodes.insert(node.id.key(), node.clone());
        }

        let mut edges = structural;
        edges.extend(resolution.edges.iter().cloned());
        edges.extend(resolution.package_edges.iter().cloned());
        edges.extend(framework.edges.iter().cloned());
        // `OVERRIDES` is read off the inheritance edges this file just produced, so it is
        // derived here rather than during reference resolution.
        let overrides = overrides::overrides_for_file(&edges, tables, &unit.file);
        edges.extend(overrides);
        edges.sort();
        edges.dedup();

        FileLinkResult {
            path: unit.file.clone(),
            nodes: file.node_inputs(),
            synthetic_nodes: synthetic_nodes.into_values().collect(),
            edges,
            unresolved: resolution.unresolved,
            deps: ResolutionDeps {
                names: resolution.consulted_names,
                files: resolution.consulted_files,
            },
            global_facts: framework.global_facts,
            fact_issues: framework.issues,
            refinements: framework.refinements,
        }
    }

    /// Links every file and folds the results into one graph-ready output.
    ///
    /// The per-file loop is the only parallel part; the merge is sequential and in path order,
    /// so the output is identical for any rayon thread count.
    #[must_use]
    pub fn link_all(input: LinkInput<'_>) -> LinkOutput {
        let LinkInput {
            units,
            resolver,
            config,
            repository_name,
        } = input;

        let (tables, names) = Self::build_tables(units);
        let results: Vec<FileLinkResult> = units
            .par_iter()
            .map(|unit| Self::link_file(unit, &tables, &names, resolver, config))
            .collect();

        let mut out = LinkOutput::default();
        let mut structure = RepositoryStructure::default();
        structure
            .nodes
            .push(structural::repository_node(repository_name));

        // Synthetic nodes are collected first: a file's refinement may target a node another
        // file contributed, so applying them per file would be order-dependent.
        let mut nodes: Vec<NodeInput> = Vec::new();
        let mut synthetic: BTreeMap<NodeKey, NodeInput> = BTreeMap::new();
        let mut edges: Vec<Edge> = Vec::new();
        let mut unresolved: Vec<UnresolvedRef> = Vec::new();
        let mut global_facts: Vec<GlobalFact> = Vec::new();
        let mut fact_issues = Vec::new();
        let mut refinements: Vec<(NodeKey, NodeKind)> = Vec::new();

        for result in &results {
            nodes.extend(result.nodes.iter().cloned());
            for node in &result.synthetic_nodes {
                synthetic.insert(node.id.key(), node.clone());
            }
            edges.extend(result.edges.iter().cloned());
            unresolved.extend(result.unresolved.iter().cloned());
            global_facts.extend(result.global_facts.iter().cloned());
            fact_issues.extend(result.fact_issues.iter().cloned());
            refinements.extend(result.refinements.iter().copied());
            structure.add_file(&result.path);
        }

        // A node with no incident edge is not evidence of anything (CG-006); drop it so the
        // graph never carries an endpoint nothing points at.
        let referenced: BTreeSet<NodeKey> = edges
            .iter()
            .flat_map(|edge| [edge.source, edge.target])
            .collect();
        synthetic.retain(|key, _| referenced.contains(key));

        nodes.extend(structure.nodes.iter().cloned());
        nodes.extend(synthetic.into_values());
        overrides::apply_refinements(&mut nodes, &refinements);
        edges.extend(structure.edges.iter().cloned());

        // Global facts run once, after every file is mapped, so an endpoint added by any file is
        // guarded even when the guard is declared in another (CG-006 `apply_globals`).
        let endpoints: Vec<(NodeKey, NodeInput)> = nodes
            .iter()
            .filter(|node| node.kind == NodeKind::ApiEndpoint)
            .map(|node| (node.id.key(), node.clone()))
            .collect();
        edges.extend(FrameworkMapper::apply_globals(&global_facts, &endpoints));
        edges.extend(overrides::overrides_for_snapshot(
            &edges,
            &tables,
            &files_in(units),
        ));

        edges.sort();
        edges.dedup();
        unresolved.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.ordinal.cmp(&b.ordinal))
                .then_with(|| a.name.cmp(&b.name))
        });
        nodes.sort_by_key(|a| a.id.key());

        out.stats = LinkStats {
            files: units.len(),
            symbols: nodes.len(),
            references: unresolved.len(),
            edges: edges.len(),
            unresolved: unresolved.len(),
            synthetic_nodes: nodes
                .iter()
                .filter(|node| node.file.is_none() && node.kind != NodeKind::Repository)
                .count(),
        };
        out.nodes = nodes;
        out.edges = edges;
        out.unresolved = unresolved;
        out.global_facts = global_facts;
        out.fact_issues = fact_issues;
        out
    }

    /// Every file the linker emitted, in path order.
    #[must_use]
    pub fn files_of(output: &LinkOutput) -> Vec<RepoPath> {
        let mut paths: BTreeSet<RepoPath> = BTreeSet::new();
        for node in &output.nodes {
            if node.kind == NodeKind::File {
                if let Some(path) = &node.file {
                    paths.insert(path.clone());
                }
            }
        }
        paths.into_iter().collect()
    }

    /// A standalone [`Edge`] for a structural relationship, used by the framework mapper and
    /// by callers that need one hand-built edge.
    #[must_use]
    pub fn structural_edge(kind: EdgeKind, source: NodeKey, target: NodeKey) -> Edge {
        Edge::new(
            kind,
            source,
            target,
            crate::confidence::confidence_of(ResolvedBy::Structural),
            ResolvedBy::Structural,
            Provenance::Analyzer,
        )
    }
}

/// Files in path order, which is the order every merge step uses.
fn files_in(units: &[Arc<ParsedUnit>]) -> Vec<RepoPath> {
    let mut paths: Vec<RepoPath> = units.iter().map(|unit| unit.file.clone()).collect();
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn link_config_defaults_match_the_cg_005_contract() {
        let config = LinkConfig::default();
        assert_eq!(config.max_ambiguous_fanout, 3);
        assert_eq!(config.max_reexport_depth, 8);
        assert_eq!(config.max_super_depth, 8);
    }

    #[test]
    fn overrides_pass_compares_kind_classes() {
        use crate::node_kind::NodeKind as K;
        assert!(overrides::same_kind_class(Some(K::Method), Some(K::Method)));
        assert!(overrides::same_kind_class(
            Some(K::Method),
            Some(K::Function)
        ));
        assert!(overrides::same_kind_class(
            Some(K::Property),
            Some(K::Field)
        ));
        assert!(!overrides::same_kind_class(Some(K::Method), Some(K::Field)));
        assert!(!overrides::same_kind_class(Some(K::Method), None));
        assert!(!overrides::same_kind_class(None, Some(K::Method)));
        assert!(!overrides::same_kind_class(
            Some(K::Class),
            Some(K::Interface)
        ));
    }

    #[test]
    fn refinements_only_apply_to_permitted_source_kinds() {
        use crate::graph::NodeInput;
        let class = NodeInput::new(
            NodeId::from_canonical("ts:src/a#C/class"),
            NodeKind::Class,
            "C",
        );
        let function = NodeInput::new(
            NodeId::from_canonical("ts:src/b#F/function"),
            NodeKind::Function,
            "F",
        );
        let class_key = class.id.key();
        let function_key = function.id.key();
        let mut nodes = vec![class, function];
        overrides::apply_refinements(
            &mut nodes,
            &[
                (class_key, NodeKind::Controller),
                (function_key, NodeKind::Middleware),
            ],
        );
        assert_eq!(nodes[0].kind, NodeKind::Controller);
        assert_eq!(nodes[1].kind, NodeKind::Middleware);

        // A refinement a kind does not accept is ignored rather than applied blindly.
        overrides::apply_refinements(&mut nodes, &[(function_key, NodeKind::Controller)]);
        assert_eq!(nodes[1].kind, NodeKind::Middleware);
    }
}
