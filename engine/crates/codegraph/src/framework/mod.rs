//! Framework facts → generic graph nodes and edges (CG-006).
//!
//! NestJS/TypeORM/BullMQ/Jest knowledge stays in `lang-typescript` (target-architecture §2.1).
//! What crosses into the graph is a category plus attributes, and this module turns that into
//! the taxonomy the rest of the system reasons about: `ApiEndpoint`, `Middleware`, `Queue`,
//! `DatabaseTable`, `TestCase`, `EnvironmentVariable` and the edges between them.
//!
//! No framework identifier appears anywhere in this module — a test greps the crate sources to
//! keep it that way, so the mapper stays reusable for any other language adapter.
//!
//! # Per-file and global facts
//!
//! [`FrameworkMapper::map_file`] is pure and runs inside the linker's parallel per-file loop.
//! Facts whose effect spans files — an application-wide guard, a global route prefix — are
//! returned as [`GlobalFact`]s and applied once by [`FrameworkMapper::apply_globals`], so an
//! incremental update can re-run them cheaply whenever the endpoint set changes.

pub mod auth;
pub mod config;
pub mod contract;
pub mod db;
pub mod http;
pub mod queue;
pub mod test;

use std::collections::BTreeMap;

use analysis_ir::framework::IrFrameworkFact;
use review_core::location::{RepoPath, SourceRange};

use crate::confidence;
use crate::edge::{Edge, EdgeFlags, Provenance, ResolvedBy};
use crate::edge_kind::EdgeKind;
use crate::graph::NodeInput;
use crate::linker::symbol_table::FileSymbols;
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;

pub use contract::{FactCategory, FactIssue, FactIssueCode};

/// A node the framework mapper invented: an endpoint, a queue, a table, an env var, a test.
///
/// The same type the graph builder takes, because the mapper produces graph inputs, not a
/// parallel model.
pub type SyntheticNode = NodeInput;

/// A fact whose effect spans files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalFact {
    /// A guard that applies to every endpoint in the repository (an `APP_GUARD` provider).
    GlobalGuard { guard: NodeKey },
    /// A route prefix declared globally; endpoints are re-keyed by the caller that owns the
    /// snapshot-wide id, because two files may contribute routes under the same prefix.
    GlobalPrefix { prefix: String },
}

/// What one file's facts produced.
#[derive(Debug, Default)]
pub struct FrameworkOutput {
    pub nodes: Vec<SyntheticNode>,
    pub edges: Vec<Edge>,
    /// `(key, refined kind)` pairs. The key never changes (clarification C3).
    pub refinements: Vec<(NodeKey, NodeKind)>,
    pub global_facts: Vec<GlobalFact>,
    pub issues: Vec<FactIssue>,
}

/// Maps framework facts onto generic graph constructs.
///
/// A namespace of pure functions over one file's facts: it holds no state between calls, so
/// `map_file` can run inside the linker's parallel per-file loop and `apply_globals` can run once
/// afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameworkMapper;

impl FrameworkMapper {
    /// Maps one file's facts. Deterministic: facts are processed in the order the adapter
    /// emitted them and job-name sets are sorted.
    #[must_use]
    pub fn map_file(
        facts: &[IrFrameworkFact],
        file: &FileSymbols,
        file_edges: &[Edge],
    ) -> FrameworkOutput {
        let mut out = FrameworkOutput::default();
        let mut job_names: BTreeMap<NodeKey, Vec<String>> = BTreeMap::new();
        let mut entity_tables: BTreeMap<String, NodeKey> = BTreeMap::new();

        // Entities first: a `db_access` fact in the same file names an entity, and the table it
        // maps to has to exist before the access edges can point at it.
        for fact in facts {
            if fact.kind == analysis_ir::framework::FrameworkFactKind::OrmEntity {
                db::index_entity(fact, file, &mut entity_tables, &mut out);
            }
        }

        for fact in facts {
            let Some(category) = contract::category_of(&fact.kind) else {
                continue;
            };
            let mut ctx = Mapping {
                category,
                fact,
                file,
                path: &file.path,
                out: &mut out,
                job_names: &mut job_names,
                entity_tables: &entity_tables,
                file_edges,
            };
            match category {
                FactCategory::Controller => http::apply_controller(&mut ctx),
                FactCategory::Route => http::apply_route(&mut ctx),
                FactCategory::Guard => auth::apply_guard(&mut ctx),
                FactCategory::QueueProducer => queue::apply_producer(&mut ctx),
                FactCategory::QueueConsumer => queue::apply_consumer(&mut ctx),
                FactCategory::Entity => db::apply_entity(&mut ctx),
                FactCategory::DbAccess => db::apply_access(&mut ctx),
                FactCategory::EnvRead => config::apply_env_read(&mut ctx),
                FactCategory::TestSuite => test::apply_suite(&mut ctx),
                FactCategory::TestCase => test::apply_case(&mut ctx),
                FactCategory::GlobalPrefix => http::apply_global_prefix(&mut ctx),
            }
        }

        // Queue job names collected while mapping become sorted node attributes.
        for (queue_key, mut names) in job_names {
            if names.is_empty() {
                continue;
            }
            names.sort();
            names.dedup();
            let Some(node) = out.nodes.iter_mut().find(|node| node.id.key() == queue_key) else {
                continue;
            };
            let joined = names.join(",");
            if node.attrs.extra.iter().any(|(key, _)| key == "jobs") {
                continue;
            }
            node.attrs.extra.push(("jobs".to_owned(), joined));
        }

        out.nodes.sort_by_key(|a| a.id.key());
        out.nodes.dedup_by(|a, b| a.id == b.id);
        out.edges.sort();
        out.edges.dedup();
        out.refinements.sort();
        out.refinements.dedup();
        out.issues.sort_by(|a, b| {
            (
                a.file.as_str(),
                a.range.start.line,
                a.range.start.column,
                a.code,
            )
                .cmp(&(
                    b.file.as_str(),
                    b.range.start.line,
                    b.range.start.column,
                    b.code,
                ))
        });
        out
    }

    /// Applies the global facts to the whole snapshot's endpoint set.
    ///
    /// `O(G × endpoints)`: a global guard authorizes every endpoint, flagged `GLOBAL_SCOPE` so a
    /// consumer can tell a repository-wide guard from a per-route one.
    #[must_use]
    pub fn apply_globals(
        globals: &[GlobalFact],
        endpoints: &[(NodeKey, SyntheticNode)],
    ) -> Vec<Edge> {
        let mut out = Vec::new();
        for global in globals {
            let GlobalFact::GlobalGuard { guard } = global else {
                continue;
            };
            for (key, _) in endpoints {
                out.push(
                    Edge::new(
                        EdgeKind::Authorizes,
                        *guard,
                        *key,
                        confidence::confidence_of(ResolvedBy::Framework),
                        ResolvedBy::Framework,
                        Provenance::Framework,
                    )
                    .with_flags(EdgeFlags::GLOBAL_SCOPE),
                );
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// One fact being mapped, plus the accumulators it may contribute to.
pub(crate) struct Mapping<'a> {
    pub category: FactCategory,
    pub fact: &'a IrFrameworkFact,
    pub file: &'a FileSymbols,
    pub path: &'a RepoPath,
    pub out: &'a mut FrameworkOutput,
    pub job_names: &'a mut BTreeMap<NodeKey, Vec<String>>,
    pub entity_tables: &'a BTreeMap<String, NodeKey>,
    pub file_edges: &'a [Edge],
}

impl Mapping<'_> {
    /// The key of a symbol this file declares, by name. `None` when the name is unknown, which
    /// the caller reports as a [`FactIssueCode::UnknownSymbol`].
    pub(crate) fn symbol_key(&mut self, name: &str) -> Option<NodeKey> {
        self.file
            .symbols
            .iter()
            .skip(1)
            .find(|symbol| symbol.name == name || symbol.qualified_name == name)
            .map(|symbol| symbol.key)
    }

    /// Like [`Self::symbol_key`], but records the issue and returns `None` on a miss.
    pub(crate) fn require_symbol(&mut self, attr: &str) -> Option<NodeKey> {
        let mut issues = Vec::new();
        let name = contract::require_str(self.fact, self.category, self.path, attr, &mut issues);
        self.out.issues.append(&mut issues);
        let name = name?;
        match self.symbol_key(&name) {
            Some(key) => Some(key),
            None => {
                self.out.issues.push(FactIssue {
                    code: FactIssueCode::UnknownSymbol,
                    category: self.category,
                    file: self.path.clone(),
                    range: self.fact.range,
                    detail: format!(
                        "attribute {attr} names {name:?}, which this file does not declare"
                    ),
                });
                None
            }
        }
    }

    /// Registers a synthetic node, or returns the key of the one already registered.
    pub(crate) fn node(
        &mut self,
        id: NodeId,
        kind: NodeKind,
        name: String,
        qualified: String,
    ) -> NodeKey {
        let key = id.key();
        if !self.out.nodes.iter().any(|node| node.id.key() == key) {
            self.out
                .nodes
                .push(NodeInput::new(id, kind, name).qualified_name(qualified));
        }
        key
    }

    /// Emits a framework edge located at the fact's range.
    pub(crate) fn edge(
        &mut self,
        kind: EdgeKind,
        source: NodeKey,
        target: NodeKey,
        flags: EdgeFlags,
    ) {
        self.edge_at(kind, source, target, flags, self.fact.range.start.line)
    }

    /// Emits a framework edge at an explicit line, for an edge derived from another edge.
    pub(crate) fn edge_at(
        &mut self,
        kind: EdgeKind,
        source: NodeKey,
        target: NodeKey,
        flags: EdgeFlags,
        line: u32,
    ) {
        let confidence = confidence::confidence_of(ResolvedBy::Framework);
        self.out.edges.push(
            Edge::new(
                kind,
                source,
                target,
                confidence,
                ResolvedBy::Framework,
                Provenance::Framework,
            )
            .with_flags(flags)
            .with_location(crate::edge::Location::new(self.path.clone(), line, 0)),
        );
    }

    /// Emits a derived edge whose confidence is the weaker of the framework rule and the edge it
    /// was read off (CG-006 "derived edges use `confidence::derived`").
    pub(crate) fn derived_edge(
        &mut self,
        kind: EdgeKind,
        source: NodeKey,
        target: NodeKey,
        input: crate::Confidence,
        line: u32,
    ) {
        self.out.edges.push(
            Edge::new(
                kind,
                source,
                target,
                confidence::derived(ResolvedBy::Framework, input),
                ResolvedBy::Framework,
                Provenance::Framework,
            )
            .with_location(crate::edge::Location::new(self.path.clone(), line, 0)),
        );
    }

    pub(crate) fn refine(&mut self, key: NodeKey, kind: NodeKind) {
        self.out.refinements.push((key, kind));
    }

    /// The current kind of a symbol, so a refinement can be chained (class → controller →
    /// nothing further, but the check is free and keeps the rule in one place).
    pub(crate) fn kind_of(&self, key: &NodeKey) -> Option<NodeKind> {
        self.file
            .symbols
            .iter()
            .find(|symbol| &symbol.key == key)
            .map(|symbol| symbol.kind)
    }

    pub(crate) fn issue(&mut self, code: FactIssueCode, detail: String) {
        self.out.issues.push(FactIssue {
            code,
            category: self.category,
            file: self.path.clone(),
            range: self.fact.range,
            detail,
        });
    }
}

/// Joins a global prefix with a route path, normalizing the result.
///
/// Shared by [`http::apply_route`] (when the adapter already applied prefixes) and by the
/// snapshot-wide id owner, so both produce the same endpoint id.
#[must_use]
pub fn join_prefix(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim().trim_end_matches('/');
    let path = path.trim();
    if prefix.is_empty() {
        return crate::node_id::normalize_http_path(path);
    }
    let joined = format!("{prefix}/{path}");
    crate::node_id::normalize_http_path(&joined)
}

/// Checks a fact's attributes against the category contract without mapping it, so an adapter
/// can self-check before emitting.
pub fn conforms(category: FactCategory, attributes: &[&str]) -> bool {
    let Some((_, required)) = FactCategory::REQUIRED
        .iter()
        .find(|(name, _)| *name == category.as_str())
    else {
        return false;
    };
    required.split(',').all(|attr| attributes.contains(&attr))
}

/// The source range a fact covers, from its `range` attribute, falling back to the fact's own
/// range.
#[must_use]
pub fn fact_range(fact: &IrFrameworkFact) -> SourceRange {
    contract::range_attr(fact.attrs.get("range")).unwrap_or(fact.range)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn prefix_join_normalizes_like_the_id_scheme() {
        assert_eq!(join_prefix("/api", "/users"), "/api/users");
        assert_eq!(join_prefix("/api/", "users"), "/api/users");
        assert_eq!(join_prefix("", "/users"), "/users");
        assert_eq!(join_prefix("/api", "/"), "/api");
        assert_eq!(join_prefix("/api", "/users/:id/"), "/api/users/{}");
    }

    #[test]
    fn contract_check_accepts_only_complete_attribute_sets() {
        assert!(conforms(FactCategory::Controller, &["symbol"]));
        assert!(!conforms(FactCategory::Controller, &[]));
        assert!(conforms(
            FactCategory::Route,
            &["symbol", "method", "path", "controller"]
        ));
        assert!(!conforms(FactCategory::Route, &["symbol", "method"]));
        assert!(conforms(
            FactCategory::DbAccess,
            &["symbol", "entity", "op"]
        ));
    }

    #[test]
    fn global_facts_only_authorize_endpoints() {
        let guard = NodeId::from_canonical("ts:src/g#G/class").key();
        let endpoint = NodeInput::new(
            NodeId::http("GET", "/users").unwrap(),
            NodeKind::ApiEndpoint,
            "GET /users",
        );
        let edges = FrameworkMapper::apply_globals(
            &[GlobalFact::GlobalGuard { guard }],
            &[(endpoint.id.key(), endpoint)],
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Authorizes);
        assert_eq!(edges[0].source, guard);
        assert!(edges[0].flags.contains(EdgeFlags::GLOBAL_SCOPE));
        assert_eq!(
            edges[0].confidence,
            confidence::confidence_of(ResolvedBy::Framework)
        );

        let prefix_only = FrameworkMapper::apply_globals(
            &[GlobalFact::GlobalPrefix {
                prefix: "/api".to_owned(),
            }],
            &[],
        );
        assert!(
            prefix_only.is_empty(),
            "a prefix re-keys endpoints, it emits no edge"
        );
    }
}
