//! Shared fixtures for the impact integration tests: a small graph builder and the PRD §151
//! auth-bypass scenario as base/head graphs plus its change set.
//!
//! The end-to-end fixture pipeline (`fixtures/pull-requests/auth-bypass`, DIFF-007/CHG-008)
//! builds the same graphs from source; until it lands this hand-built pair stands in for it,
//! with node ids, kinds and edge confidences exactly as the linker and framework mapper emit
//! them.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use codegraph::{
    Confidence, Edge, EdgeFlags, EdgeKind, Graph, GraphBuilder, NodeFlags, NodeId, NodeInput,
    NodeInputAttrs, NodeKey, NodeKind, Provenance, ResolvedBy, SCHEMA_VERSION,
};
use impact::graph::{ImpactBudget, ImpactGraph, ImpactInputs};
use impact::input::{tags, CallInput, ChangeClass, ChangeSet, ClassInput, FileInput, SymbolInput};
use review_core::change::{ChangedSymbol, FileChangeStatus, SymbolChange};
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::{DiffSide, Position, RepoPath, SourceRange};

pub fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

pub fn range(start: u32, end: u32) -> SourceRange {
    SourceRange {
        start: Position {
            line: start,
            column: 0,
        },
        end: Position {
            line: end,
            column: 0,
        },
    }
}

/// The IR kind spelled in a symbol id; refined framework kinds keep their IR kind.
fn suffix(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Method | NodeKind::Handler | NodeKind::QueueProducer | NodeKind::JobHandler => {
            "method"
        }
        NodeKind::Class
        | NodeKind::Controller
        | NodeKind::DatabaseEntity
        | NodeKind::Middleware
        | NodeKind::QueueConsumer => "class",
        NodeKind::Interface => "interface",
        NodeKind::Constructor => "constructor",
        NodeKind::Property => "property",
        NodeKind::Variable => "variable",
        NodeKind::Constant => "constant",
        NodeKind::TypeAlias => "type",
        NodeKind::Enum => "enum",
        _ => "function",
    }
}

/// `ts:{file without extension}#{qualified}/{kind}`.
pub fn sym_id(file: &str, qualified: &str, kind: NodeKind) -> String {
    let module = file
        .rsplit_once('.')
        .map_or(file, |(stem, _)| stem)
        .to_owned();
    format!("ts:{module}#{qualified}/{}", suffix(kind))
}

pub fn sym_key(file: &str, qualified: &str, kind: NodeKind) -> NodeKey {
    NodeId::from_canonical(sym_id(file, qualified, kind)).key()
}

pub fn conf(value: f32) -> Confidence {
    Confidence::from_f32(value)
}

/// A tiny fluent graph builder.
pub struct Fixture {
    builder: GraphBuilder,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            builder: GraphBuilder::new(SCHEMA_VERSION),
        }
    }

    pub fn node_with(
        &mut self,
        id: NodeId,
        kind: NodeKind,
        name: &str,
        qualified: &str,
        file: Option<&str>,
        attrs: NodeInputAttrs,
    ) -> NodeKey {
        let key = id.key();
        let mut input = NodeInput::new(id, kind, name)
            .qualified_name(qualified)
            .with_attrs(attrs);
        if let Some(file) = file {
            input = input.in_file(path(file));
        }
        self.builder.add_node(input).unwrap();
        key
    }

    /// A top-level symbol of `file`.
    pub fn symbol(&mut self, file: &str, qualified: &str, kind: NodeKind) -> NodeKey {
        self.symbol_flags(file, qualified, kind, NodeFlags::EMPTY)
    }

    pub fn symbol_flags(
        &mut self,
        file: &str,
        qualified: &str,
        kind: NodeKind,
        flags: NodeFlags,
    ) -> NodeKey {
        let name = qualified.rsplit('.').next().unwrap_or(qualified).to_owned();
        let attrs = NodeInputAttrs {
            flags,
            ..NodeInputAttrs::default()
        };
        self.node_with(
            NodeId::from_canonical(sym_id(file, qualified, kind)),
            kind,
            &name,
            qualified,
            Some(file),
            attrs,
        )
    }

    /// A member `Parent.name` of `parent`, with the `CONTAINS` edge.
    pub fn member(
        &mut self,
        file: &str,
        parent: NodeKey,
        qualified: &str,
        kind: NodeKind,
    ) -> NodeKey {
        let name = qualified.rsplit('.').next().unwrap_or(qualified).to_owned();
        let attrs = NodeInputAttrs {
            parent: Some(parent),
            ..NodeInputAttrs::default()
        };
        let key = self.node_with(
            NodeId::from_canonical(sym_id(file, qualified, kind)),
            kind,
            &name,
            qualified,
            Some(file),
            attrs,
        );
        self.edge(EdgeKind::Contains, parent, key, ResolvedBy::Structural);
        key
    }

    /// The `File` node of `file`, contained by its directory node.
    pub fn file(&mut self, file: &str) -> NodeKey {
        let repo_path = path(file);
        let key = self.node_with(
            NodeId::file(&repo_path),
            NodeKind::File,
            file.rsplit('/').next().unwrap_or(file),
            file,
            Some(file),
            NodeInputAttrs::default(),
        );
        let directory = NodeId::containing_directory(&repo_path);
        let dir_name = directory
            .as_str()
            .strip_prefix("dir:")
            .unwrap_or(directory.as_str())
            .to_owned();
        let dir_key = self.node_with(
            directory,
            NodeKind::Directory,
            &dir_name,
            &dir_name,
            None,
            NodeInputAttrs::default(),
        );
        self.edge(EdgeKind::Contains, dir_key, key, ResolvedBy::Structural);
        key
    }

    /// A synthetic node (endpoint, table, queue, env var, package).
    pub fn synthetic(
        &mut self,
        id: NodeId,
        kind: NodeKind,
        name: &str,
        qualified: &str,
    ) -> NodeKey {
        self.node_with(id, kind, name, qualified, None, NodeInputAttrs::default())
    }

    /// `http:{METHOD} {path}` with the framework mapper's naming.
    pub fn endpoint(&mut self, method: &str, route: &str) -> NodeKey {
        let id = NodeId::http(method, route).unwrap();
        let display = id.as_str().trim_start_matches("http:").to_owned();
        self.synthetic(
            id,
            NodeKind::ApiEndpoint,
            &display,
            &format!("{} {route}", method.to_ascii_uppercase()),
        )
    }

    pub fn table(&mut self, table: &str) -> NodeKey {
        let id = NodeId::table(None, table).unwrap();
        let display = id.as_str().trim_start_matches("db:").to_owned();
        self.synthetic(id, NodeKind::DatabaseTable, &display, &display)
    }

    pub fn queue(&mut self, name: &str) -> NodeKey {
        self.synthetic(NodeId::queue(name).unwrap(), NodeKind::Queue, name, name)
    }

    pub fn env(&mut self, name: &str) -> NodeKey {
        self.synthetic(
            NodeId::env(name).unwrap(),
            NodeKind::EnvironmentVariable,
            name,
            name,
        )
    }

    pub fn package(&mut self, spec: &str) -> NodeKey {
        self.synthetic(
            NodeId::package("npm", spec).unwrap(),
            NodeKind::ExternalDependency,
            spec,
            spec,
        )
    }

    /// A test case `suite › name` of `file`, inside its suite node.
    pub fn test_case(&mut self, file: &str, suite: &str, name: &str) -> NodeKey {
        let repo_path = path(file);
        let suite_id = NodeId::test(&repo_path, "", suite).unwrap();
        let suite_key = self.node_with(
            suite_id.clone(),
            NodeKind::TestSuite,
            suite,
            suite_id.as_str(),
            Some(file),
            NodeInputAttrs::default(),
        );
        let case_id = NodeId::test(&repo_path, suite, name).unwrap();
        let attrs = NodeInputAttrs {
            flags: NodeFlags::TEST,
            ..NodeInputAttrs::default()
        };
        let case_key = self.node_with(
            case_id.clone(),
            NodeKind::TestCase,
            name,
            case_id.as_str(),
            Some(file),
            attrs,
        );
        self.edge(
            EdgeKind::Contains,
            suite_key,
            case_key,
            ResolvedBy::Structural,
        );
        case_key
    }

    pub fn edge(&mut self, kind: EdgeKind, from: NodeKey, to: NodeKey, by: ResolvedBy) {
        self.edge_conf(kind, from, to, codegraph::confidence_of(by), by);
    }

    pub fn edge_conf(
        &mut self,
        kind: EdgeKind,
        from: NodeKey,
        to: NodeKey,
        confidence: Confidence,
        by: ResolvedBy,
    ) {
        self.builder.add_edge(Edge::new(
            kind,
            from,
            to,
            confidence,
            by,
            Provenance::Linker,
        ));
    }

    pub fn edge_flags(
        &mut self,
        kind: EdgeKind,
        from: NodeKey,
        to: NodeKey,
        by: ResolvedBy,
        flags: EdgeFlags,
    ) {
        self.builder.add_edge(
            Edge::new(
                kind,
                from,
                to,
                codegraph::confidence_of(by),
                by,
                Provenance::Framework,
            )
            .with_flags(flags),
        );
    }

    pub fn build(self) -> Graph {
        self.builder.build().unwrap()
    }
}

/// A `ChangedSymbol` on head (or base for `Removed`).
pub fn changed(file: &str, qualified: &str, kind: NodeKind, change: SymbolChange) -> SymbolInput {
    let id = sym_id(file, qualified, kind);
    let side = if matches!(change, SymbolChange::Removed) {
        DiffSide::Base
    } else {
        DiffSide::Head
    };
    let symbol_id = SymbolId::from_canonical_unchecked(id);
    let mut input = SymbolInput::new(ChangedSymbol {
        symbol_key: SymbolKey::of(&symbol_id),
        symbol_id,
        path: path(file),
        side,
        range: range(10, 20),
        change,
    });
    input.kind = Some(kind);
    input
}

pub fn body_change() -> SymbolChange {
    SymbolChange::modified(false, true, false).unwrap()
}

pub fn modified_file(file: &str) -> FileInput {
    FileInput::new(path(file), FileChangeStatus::Modified)
}

/// Builds the impact graph with the default budget, sequentially.
pub fn impact_of(change: &ChangeSet, head: &Graph, base: Option<&Graph>) -> ImpactGraph {
    impact_with(change, head, base, &ImpactBudget::default())
}

pub fn impact_with(
    change: &ChangeSet,
    head: &Graph,
    base: Option<&Graph>,
    budget: &ImpactBudget,
) -> ImpactGraph {
    impact::build_impact(&ImpactInputs {
        change,
        head,
        base: base.map(|graph| graph as &dyn codegraph::GraphQuery),
        budget,
        priority: None,
        parallel: false,
    })
}

// ---------------------------------------------------------------------------------------------
// The PRD §151 auth-bypass scenario.
// ---------------------------------------------------------------------------------------------

pub const PROVIDER: &str = "src/auth/auth-provider.interface.ts";
pub const AUTH_SERVICE: &str = "src/auth/auth.service.ts";
pub const PERMISSION_SERVICE: &str = "src/auth/permission.service.ts";
pub const ADMIN_SERVICE: &str = "src/admin/admin.service.ts";
pub const USER_CONTROLLER: &str = "src/users/user.controller.ts";
pub const USER_ENTITY: &str = "src/users/user.entity.ts";
pub const AUTH_SPEC: &str = "src/auth/authorize.spec.ts";
pub const REPORT_SERVICE: &str = "src/reports/report.service.ts";
pub const FORMAT: &str = "src/util/format.ts";

pub const AUTHORIZE_ID: &str = "ts:src/auth/auth.service#AuthService.authorize/method";
pub const PERMISSION_CHECK_ID: &str =
    "ts:src/auth/permission.service#PermissionService.check/method";
pub const TEST_CASE_ID: &str =
    "test:src/auth/authorize.spec.ts#AuthService › denies without permission";

/// Keys of the scenario's interesting nodes.
#[derive(Debug, Clone, Copy)]
pub struct AuthBypassKeys {
    pub authorize: NodeKey,
    pub provider_authorize: NodeKey,
    pub permission_check: NodeKey,
    pub update_user: NodeKey,
    pub controller_update: NodeKey,
    pub endpoint: NodeKey,
    pub users_table: NodeKey,
    pub report_generate: NodeKey,
    pub authorize_header: NodeKey,
    pub test_case: NodeKey,
}

pub struct AuthBypass {
    pub head: Graph,
    pub base: Graph,
    pub change: ChangeSet,
    pub keys: AuthBypassKeys,
}

fn auth_bypass_graph(base_side: bool) -> (Graph, AuthBypassKeys) {
    let mut g = Fixture::new();
    for file in [
        PROVIDER,
        AUTH_SERVICE,
        PERMISSION_SERVICE,
        ADMIN_SERVICE,
        USER_CONTROLLER,
        USER_ENTITY,
        AUTH_SPEC,
        REPORT_SERVICE,
        FORMAT,
    ] {
        g.file(file);
    }

    let provider = g.symbol_flags(
        PROVIDER,
        "AuthProvider",
        NodeKind::Interface,
        NodeFlags::EXPORTED,
    );
    let provider_authorize = g.member(
        PROVIDER,
        provider,
        "AuthProvider.authorize",
        NodeKind::Method,
    );

    let auth_service = g.symbol_flags(
        AUTH_SERVICE,
        "AuthService",
        NodeKind::Class,
        NodeFlags::EXPORTED,
    );
    let authorize = g.member(
        AUTH_SERVICE,
        auth_service,
        "AuthService.authorize",
        NodeKind::Method,
    );
    g.edge(
        EdgeKind::Implements,
        auth_service,
        provider,
        ResolvedBy::Import,
    );
    g.edge(
        EdgeKind::Overrides,
        authorize,
        provider_authorize,
        ResolvedBy::Structural,
    );

    let permission_service = g.symbol_flags(
        PERMISSION_SERVICE,
        "PermissionService",
        NodeKind::Class,
        NodeFlags::EXPORTED,
    );
    let permission_check = g.member(
        PERMISSION_SERVICE,
        permission_service,
        "PermissionService.check",
        NodeKind::Method,
    );
    if base_side {
        g.edge(
            EdgeKind::Calls,
            authorize,
            permission_check,
            ResolvedBy::DiConstructor,
        );
    }

    let users_table = g.table("users");
    let user_entity = g.symbol_flags(
        USER_ENTITY,
        "User",
        NodeKind::DatabaseEntity,
        NodeFlags::EXPORTED,
    );
    g.edge_flags(
        EdgeKind::References,
        user_entity,
        users_table,
        ResolvedBy::Framework,
        EdgeFlags::MAPS_TABLE,
    );

    let admin_service = g.symbol_flags(
        ADMIN_SERVICE,
        "AdminService",
        NodeKind::Class,
        NodeFlags::EXPORTED,
    );
    let update_user = g.member(
        ADMIN_SERVICE,
        admin_service,
        "AdminService.updateUser",
        NodeKind::Method,
    );
    g.edge(
        EdgeKind::Calls,
        update_user,
        authorize,
        ResolvedBy::DiConstructor,
    );
    g.edge(
        EdgeKind::WritesTable,
        update_user,
        users_table,
        ResolvedBy::Framework,
    );

    let controller = g.symbol_flags(
        USER_CONTROLLER,
        "UserController",
        NodeKind::Controller,
        NodeFlags::EXPORTED,
    );
    let controller_update = g.member(
        USER_CONTROLLER,
        controller,
        "UserController.update",
        NodeKind::Handler,
    );
    g.edge(
        EdgeKind::Calls,
        controller_update,
        update_user,
        ResolvedBy::DiConstructor,
    );
    let endpoint = g.endpoint("PUT", "/users/:id");
    g.edge(
        EdgeKind::HandledBy,
        endpoint,
        controller_update,
        ResolvedBy::Framework,
    );
    g.edge(
        EdgeKind::RoutesTo,
        endpoint,
        controller,
        ResolvedBy::Framework,
    );

    // The spec imports AuthService and its case exercises authorize; PermissionService is mocked.
    let test_case = g.test_case(AUTH_SPEC, "AuthService", "denies without permission");
    g.edge(EdgeKind::Tests, test_case, authorize, ResolvedBy::Framework);
    let spec_file = NodeId::file(&path(AUTH_SPEC)).key();
    let service_file = NodeId::file(&path(AUTH_SERVICE)).key();
    g.edge(
        EdgeKind::Imports,
        spec_file,
        service_file,
        ResolvedBy::Import,
    );

    // Decoys: an independent caller of PermissionService.check, and a lexically similar name.
    let report_service = g.symbol(REPORT_SERVICE, "ReportService", NodeKind::Class);
    let report_generate = g.member(
        REPORT_SERVICE,
        report_service,
        "ReportService.generate",
        NodeKind::Method,
    );
    g.edge(
        EdgeKind::Calls,
        report_generate,
        permission_check,
        ResolvedBy::DiConstructor,
    );
    let authorize_header = g.symbol_flags(
        FORMAT,
        "authorizeHeader",
        NodeKind::Function,
        NodeFlags::EXPORTED,
    );

    let keys = AuthBypassKeys {
        authorize,
        provider_authorize,
        permission_check,
        update_user,
        controller_update,
        endpoint,
        users_table,
        report_generate,
        authorize_header,
        test_case,
    };
    (g.build(), keys)
}

/// The §151 change: `authorize` stops calling `PermissionService.check` and compares a role.
pub fn auth_bypass_change() -> ChangeSet {
    let mut authorize = changed(
        AUTH_SERVICE,
        "AuthService.authorize",
        NodeKind::Method,
        body_change(),
    );
    authorize.exported = true;
    authorize.classes = vec![
        ClassInput::new(ChangeClass::ConditionChanged),
        ClassInput::new(ChangeClass::ReturnChanged),
        ClassInput::new(ChangeClass::CallRemoved),
        ClassInput::new(ChangeClass::DependencyRemoved),
        ClassInput::new(ChangeClass::AuthorizationChanged)
            .tag(tags::AUTHZ_CALL_REMOVED)
            .tag(tags::DIRECT_ROLE_COMPARISON_ADDED),
    ];
    authorize.removed_calls = vec![CallInput::to(
        PERMISSION_CHECK_ID,
        "this.permissionService.check",
    )];
    authorize.size_lines = 6;

    let mut format = modified_file(FORMAT);
    format.cosmetic_only = true;
    format.additions = 1;
    format.deletions = 1;
    let mut service = modified_file(AUTH_SERVICE);
    service.additions = 4;
    service.deletions = 3;

    ChangeSet {
        input_hash: "auth-bypass-change-model".to_owned(),
        head_snapshot: "head".to_owned(),
        base_snapshot: "base".to_owned(),
        files: vec![service, format],
        symbols: vec![authorize],
        ..ChangeSet::default()
    }
}

pub fn auth_bypass() -> AuthBypass {
    let (head, keys) = auth_bypass_graph(false);
    let (base, _) = auth_bypass_graph(true);
    AuthBypass {
        head,
        base,
        change: auth_bypass_change(),
        keys,
    }
}
