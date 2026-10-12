//! A tiny DSL for synthetic graphs, and the layered NestJS-shaped repository the profile and
//! policy tests share.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::facts::{FactKind, SyntaxFact};
use analysis_ir::symbol::{AttrValue, Visibility};
use codegraph::{
    confidence_of, Edge, EdgeFlags, EdgeKind, FileInput, Graph, GraphBuilder, Location, NodeFlags,
    NodeId, NodeInput, NodeInputAttrs, NodeKey, NodeKind, Provenance, ResolvedBy, SCHEMA_VERSION,
};
use profile::facts::FactIndex;
use review_core::language::Language;
use review_core::location::{ContentHash, Position, RepoPath, SourceRange};

pub fn range(start: u32, end: u32) -> SourceRange {
    SourceRange::new(
        Position::new(start, 0).unwrap(),
        Position::new(end.max(start), 1).unwrap(),
    )
    .unwrap()
}

pub fn fact(kind: FactKind, start: u32, end: u32, detail: &[(&str, &str)]) -> SyntaxFact {
    let detail: BTreeMap<String, AttrValue> = detail
        .iter()
        .map(|(k, v)| {
            let value = match *v {
                "true" => AttrValue::Bool(true),
                other => AttrValue::Str(other.to_owned()),
            };
            ((*k).to_owned(), value)
        })
        .collect();
    SyntaxFact {
        kind,
        key: format!("{kind:?}:{start}"),
        range: range(start, end),
        detail,
    }
}

/// Graph builder with sensible defaults.
#[derive(Default)]
pub struct G {
    files: BTreeSet<String>,
    nodes: Vec<NodeInput>,
    edges: Vec<Edge>,
    pub facts: FactIndex,
}

pub struct Spec<'a> {
    pub path: &'a str,
    pub name: &'a str,
    pub kind: NodeKind,
    pub line: u32,
    pub exported: bool,
    pub parent: Option<NodeKey>,
    pub private: bool,
}

impl G {
    pub fn new() -> Self {
        Self::default()
    }

    fn file(&mut self, path: &str) {
        if self.files.insert(path.to_owned()) {
            self.nodes.push(
                NodeInput::new(
                    NodeId::file(&RepoPath::new(path).unwrap()),
                    NodeKind::File,
                    path.rsplit('/').next().unwrap(),
                )
                .qualified_name(path)
                .in_file(RepoPath::new(path).unwrap()),
            );
        }
    }

    pub fn file_key(&mut self, path: &str) -> NodeKey {
        self.file(path);
        NodeId::file(&RepoPath::new(path).unwrap()).key()
    }

    pub fn add(&mut self, spec: Spec<'_>) -> NodeKey {
        self.file(spec.path);
        let module = spec.path.rsplit_once('.').map_or(spec.path, |(m, _)| m);
        let id = format!("ts:{module}#{}/{}", spec.name, spec.kind.as_str().to_lowercase());
        let id = NodeId::from_canonical(id);
        let key = id.key();
        let mut flags = NodeFlags::EMPTY;
        if spec.exported {
            flags = flags.union(NodeFlags::EXPORTED);
        }
        let attrs = NodeInputAttrs {
            visibility: if spec.private {
                Visibility::Private
            } else {
                Visibility::Public
            },
            flags,
            parent: spec.parent,
            ..NodeInputAttrs::default()
        };
        let short = spec.name.rsplit('.').next().unwrap_or(spec.name);
        self.nodes.push(
            NodeInput::new(id, spec.kind, short)
                .qualified_name(spec.name)
                .in_file(RepoPath::new(spec.path).unwrap())
                .with_range(range(spec.line, spec.line + 5))
                .with_attrs(attrs),
        );
        key
    }

    /// An exported class.
    pub fn class(&mut self, path: &str, name: &str, kind: NodeKind) -> NodeKey {
        self.add(Spec {
            path,
            name,
            kind,
            line: 1,
            exported: true,
            parent: None,
            private: false,
        })
    }

    /// A public method of `parent`.
    pub fn method(&mut self, path: &str, qualified: &str, parent: NodeKey, line: u32) -> NodeKey {
        self.add(Spec {
            path,
            name: qualified,
            kind: NodeKind::Method,
            line,
            exported: false,
            parent: Some(parent),
            private: false,
        })
    }

    /// A node with no file (endpoint, queue, ...).
    pub fn synthetic(&mut self, id: &str, kind: NodeKind, name: &str) -> NodeKey {
        let node_id = NodeId::from_canonical(id.to_owned());
        let key = node_id.key();
        self.nodes
            .push(NodeInput::new(node_id, kind, name).qualified_name(name));
        key
    }

    pub fn edge(&mut self, kind: EdgeKind, from: NodeKey, to: NodeKey) {
        self.edges.push(Edge::new(
            kind,
            from,
            to,
            confidence_of(ResolvedBy::NameUnique),
            ResolvedBy::NameUnique,
            Provenance::Linker,
        ));
    }

    pub fn edge_with(&mut self, kind: EdgeKind, from: NodeKey, to: NodeKey, resolved: ResolvedBy, flags: EdgeFlags) {
        self.edges.push(
            Edge::new(kind, from, to, confidence_of(resolved), resolved, Provenance::Linker)
                .with_flags(flags),
        );
    }

    /// An edge observed at `path:line`.
    pub fn edge_at(&mut self, kind: EdgeKind, from: NodeKey, to: NodeKey, path: &str, line: u32) {
        self.edges.push(
            Edge::new(
                kind,
                from,
                to,
                confidence_of(ResolvedBy::Import),
                ResolvedBy::Import,
                Provenance::Linker,
            )
            .with_location(Location::new(RepoPath::new(path).unwrap(), line, 1)),
        );
    }

    pub fn facts(&mut self, key: NodeKey, facts: Vec<SyntaxFact>) {
        self.facts.insert(key, facts);
    }

    pub fn build(self) -> (Graph, FactIndex) {
        let mut builder = GraphBuilder::new(SCHEMA_VERSION);
        for path in &self.files {
            builder
                .add_file(FileInput {
                    path: RepoPath::new(path.clone()).unwrap(),
                    file_version_id: None,
                    content_hash: ContentHash::of(path.as_bytes()),
                    language: Language::Typescript,
                })
                .unwrap();
        }
        for node in self.nodes {
            builder.add_node(node).unwrap();
        }
        for edge in self.edges {
            builder.add_edge(edge);
        }
        (builder.build().unwrap(), self.facts)
    }
}

/// Keys of the layered fixture.
#[derive(Debug, Clone, Default)]
pub struct Layered {
    pub keys: BTreeMap<String, NodeKey>,
}

impl Layered {
    pub fn key(&self, name: &str) -> NodeKey {
        *self.keys.get(name).unwrap_or_else(|| panic!("no node {name}"))
    }
}

/// A synthetic NestJS-shaped layered repository: three feature modules, each with a controller,
/// a service, a repository and an entity, plus a guard, endpoints, a queue and tests. Planted:
/// `OrdersController` depends on `OrdersRepository`, and `BillingService.charge` writes outside a
/// transaction.
pub fn layered() -> (G, Layered) {
    let mut g = G::new();
    let mut l = Layered::default();
    let guard = g.class("src/common/auth.guard.ts", "AuthGuard", NodeKind::Middleware);
    l.keys.insert("AuthGuard".into(), guard);
    let base_error = g.class("src/common/domain.error.ts", "DomainError", NodeKind::Class);
    l.keys.insert("DomainError".into(), base_error);
    for module in ["users", "orders", "billing"] {
        let cap = format!("{}{}", module[..1].to_uppercase(), &module[1..]);
        let dir = format!("src/modules/{module}");
        let ctrl_path = format!("{dir}/{module}.controller.ts");
        let svc_path = format!("{dir}/{module}.service.ts");
        let repo_path = format!("{dir}/{module}.repository.ts");
        let entity_path = format!("{dir}/{module}.entity.ts");
        let ctrl = g.class(&ctrl_path, &format!("{cap}Controller"), NodeKind::Controller);
        let handler = g.method(&ctrl_path, &format!("{cap}Controller.list"), ctrl, 10);
        let svc = g.class(&svc_path, &format!("{cap}Service"), NodeKind::Class);
        let create = g.method(&svc_path, &format!("{cap}Service.create"), svc, 10);
        let charge = g.method(&svc_path, &format!("{cap}Service.charge"), svc, 30);
        let repo = g.class(&repo_path, &format!("{cap}Repository"), NodeKind::Class);
        let save = g.method(&repo_path, &format!("{cap}Repository.save"), repo, 5);
        let entity = g.class(&entity_path, &cap, NodeKind::DatabaseEntity);
        let error = g.class(&format!("{dir}/{module}.error.ts"), &format!("{cap}Error"), NodeKind::Class);
        g.edge(EdgeKind::Extends, error, base_error);

        g.edge_at(EdgeKind::DependsOn, ctrl, svc, &ctrl_path, 3);
        g.edge_at(EdgeKind::Calls, handler, create, &ctrl_path, 11);
        g.edge_at(EdgeKind::DependsOn, svc, repo, &svc_path, 3);
        g.edge_at(EdgeKind::Calls, create, save, &svc_path, 12);
        g.edge(EdgeKind::UsesType, repo, entity);
        if module == "orders" {
            // Planted: a controller reaching a repository.
            g.edge_at(EdgeKind::DependsOn, ctrl, repo, &ctrl_path, 4);
        }
        let endpoint = g.synthetic(
            &format!("http:GET /v1/{module}"),
            NodeKind::ApiEndpoint,
            &format!("GET /v1/{module}"),
        );
        g.edge(EdgeKind::HandledBy, endpoint, handler);
        g.edge(EdgeKind::RoutesTo, endpoint, ctrl);
        g.edge_with(EdgeKind::Authorizes, guard, endpoint, ResolvedBy::Framework, EdgeFlags::GLOBAL_SCOPE);

        // create: write inside a transaction; charge: write, wrapped except in billing.
        g.facts(
            create,
            vec![
                fact(FactKind::TransactionWrapper, 11, 15, &[]),
                fact(FactKind::DbWriteLike, 12, 12, &[]),
                fact(FactKind::Throw, 13, 13, &[("class", &format!("{cap}Error"))]),
            ],
        );
        let mut charge_facts = vec![fact(FactKind::DbWriteLike, 32, 32, &[])];
        if module != "billing" {
            charge_facts.insert(0, fact(FactKind::TransactionWrapper, 31, 34, &[]));
        }
        g.facts(charge, charge_facts);

        let spec = format!("{dir}/{module}.service.spec.ts");
        let case = g.synthetic(
            &format!("test:{spec}#{cap}Service › creates"),
            NodeKind::TestCase,
            &format!("{cap}Service creates"),
        );
        g.edge(EdgeKind::Tests, case, create);
        let _ = g.file_key(&spec);

        for (name, key) in [
            (format!("{cap}Controller"), ctrl),
            (format!("{cap}Controller.list"), handler),
            (format!("{cap}Service"), svc),
            (format!("{cap}Service.create"), create),
            (format!("{cap}Service.charge"), charge),
            (format!("{cap}Repository"), repo),
            (format!("{cap}Repository.save"), save),
            (cap.clone(), entity),
            (format!("endpoint:{module}"), endpoint),
        ] {
            l.keys.insert(name, key);
        }
    }
    (g, l)
}
