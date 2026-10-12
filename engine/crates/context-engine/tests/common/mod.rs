//! Shared builders for the context-engine tests: a synthetic "auth bypass" pull request.
//!
//! `AuthService.authorize` is changed (a `PermissionService.check` call was removed), its caller
//! `AdminService.updateUser` and the controller `UserController.update` sit in front of the
//! endpoint `PUT /users/:id`, and `ReportService.generate` is an unrelated decoy.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::str::FromStr;

use codegraph::{Confidence, EdgeKind, NodeId, NodeKind};
use context_engine::candidates::{
    ChangedSeed, NodeInfo, NodeLocator, SeedContainer, SelectionInput,
};
use context_engine::compress::BlobSource;
use impact::graph::{
    EndpointAttrs, EntryKind, GraphSide, ImpactBudget, ImpactElement, ImpactFlags, ImpactGraph,
    ImpactStats, PathStep, SymbolImpact, TestMapping, TestSignals,
};
use impact::Relation;
use review_core::ids::{CommitSha, SymbolKey};
use review_core::location::{DiffSide, RepoPath};
use review_core::reviewer_type::ReviewerType;

pub const AUTHORIZE: &str = "ts:src/auth/auth.service.ts#AuthService.authorize";
pub const AUTH_CLASS: &str = "ts:src/auth/auth.service.ts#AuthService";
pub const UPDATE_USER: &str = "ts:src/admin/admin.service.ts#AdminService.updateUser";
pub const CONTROLLER: &str = "ts:src/users/user.controller.ts#UserController.update";
pub const ENDPOINT: &str = "http:PUT /users/:id";
pub const PROVIDER: &str = "ts:src/auth/auth.provider.ts#AuthProvider.authorize";
pub const PERMISSION_CHECK: &str = "ts:src/auth/permission.service.ts#PermissionService.check";
pub const SPEC: &str = "ts:src/auth/auth.service.spec.ts#authorize denies";
pub const DECOY: &str = "ts:src/reports/report.service.ts#ReportService.generate";
pub const FORMAT_HEADER: &str = "ts:src/util/format.ts#authorizeHeader";

pub fn key(id: &str) -> SymbolKey {
    NodeId::from_canonical(id).key()
}

pub fn head() -> CommitSha {
    CommitSha::from_str(&"b".repeat(40)).unwrap()
}

pub fn base() -> CommitSha {
    CommitSha::from_str(&"a".repeat(40)).unwrap()
}

pub fn path(p: &str) -> RepoPath {
    RepoPath::new(p).unwrap()
}

fn step(from: &str, edge: EdgeKind, to: &str, conf: f32, graph: GraphSide) -> PathStep {
    PathStep {
        from: key(from),
        edge,
        to: key(to),
        confidence: Confidence::from_f32(conf),
        graph,
    }
}

pub fn element(
    id: &str,
    kind: NodeKind,
    relation: Relation,
    distance: u8,
    conf: f32,
) -> ImpactElement {
    ImpactElement {
        node: key(id),
        node_id: id.to_owned(),
        kind,
        relation,
        distance,
        path: vec![step(id, EdgeKind::Calls, AUTHORIZE, conf, GraphSide::Head)],
        min_confidence: Confidence::from_f32(conf),
        alt_paths: 0,
        weak: false,
        endpoint: None,
        test: None,
        resource: None,
        touches: Vec::new(),
        missing_on_head: false,
    }
}

pub fn endpoint_element() -> ImpactElement {
    let mut e = element(ENDPOINT, NodeKind::ApiEndpoint, Relation::Endpoint, 3, 0.95);
    e.path = vec![
        step(
            UPDATE_USER,
            EdgeKind::Calls,
            AUTHORIZE,
            1.0,
            GraphSide::Head,
        ),
        step(
            CONTROLLER,
            EdgeKind::Calls,
            UPDATE_USER,
            0.95,
            GraphSide::Head,
        ),
        step(
            ENDPOINT,
            EdgeKind::HandledBy,
            CONTROLLER,
            0.95,
            GraphSide::Head,
        ),
    ];
    e.endpoint = Some(EndpointAttrs {
        entry_kind: EntryKind::Http,
        method: Some("PUT".to_owned()),
        path: Some("/users/:id".to_owned()),
        guards: vec!["JwtGuard".to_owned()],
    });
    e
}

pub fn test_element(score: f32, mocked: bool) -> ImpactElement {
    let mut e = element(SPEC, NodeKind::TestCase, Relation::Test, 1, 1.0);
    e.test = Some(TestMapping {
        score,
        signals: TestSignals::default(),
        mocked,
    });
    e
}

pub fn removed_callee() -> ImpactElement {
    let mut e = element(
        PERMISSION_CHECK,
        NodeKind::Method,
        Relation::RemovedCallee,
        1,
        1.0,
    );
    e.path = vec![step(
        AUTHORIZE,
        EdgeKind::Calls,
        PERMISSION_CHECK,
        1.0,
        GraphSide::Base,
    )];
    e
}

pub fn auth_bypass_elements() -> Vec<ImpactElement> {
    vec![
        element(UPDATE_USER, NodeKind::Method, Relation::Caller, 1, 1.0),
        element(CONTROLLER, NodeKind::Method, Relation::Caller, 2, 0.95),
        element(PROVIDER, NodeKind::Method, Relation::Implementation, 1, 0.9),
        removed_callee(),
        endpoint_element(),
        test_element(0.9, false),
    ]
}

pub fn graph_of(elements: Vec<ImpactElement>) -> ImpactGraph {
    ImpactGraph {
        schema_version: 1,
        symbols: vec![SymbolImpact {
            seed: key(AUTHORIZE),
            seed_id: AUTHORIZE.to_owned(),
            side: GraphSide::Head,
            skipped: None,
            untested: false,
            elements,
            truncation: Vec::new(),
        }],
        budget: ImpactBudget::default(),
        stats: ImpactStats::default(),
        flags: ImpactFlags::default(),
        test_targets: Vec::new(),
        input_hash: "0".repeat(64),
    }
}

pub fn seed() -> ChangedSeed {
    ChangedSeed {
        key: key(AUTHORIZE),
        title: AUTHORIZE.to_owned(),
        file: path("src/auth/auth.service.ts"),
        side: DiffSide::Head,
        start_line: 10,
        end_line: 30,
        body_changed: true,
        base_range: Some((10, 34)),
        removed: false,
        is_test: false,
        skipped: false,
        risk: 0.9,
        container: Some(SeedContainer {
            key: key(AUTH_CLASS),
            title: AUTH_CLASS.to_owned(),
            start_line: 1,
            end_line: 80,
        }),
    }
}

pub fn input<'a>(reviewer: ReviewerType, graph: Option<&'a ImpactGraph>) -> SelectionInput<'a> {
    let mut input = SelectionInput::new(reviewer, head(), base());
    input.seeds = vec![seed()];
    input.impact = graph;
    input
}

/// Locator over a fixed table.
#[derive(Debug, Default)]
pub struct TableLocator(pub BTreeMap<SymbolKey, NodeInfo>);

impl NodeLocator for TableLocator {
    fn locate(&self, node: SymbolKey, _side: DiffSide) -> Option<NodeInfo> {
        self.0.get(&node).cloned()
    }
}

/// Blob source over `(commit, path) -> text`.
#[derive(Debug, Default)]
pub struct MapBlobs(pub BTreeMap<(String, String), String>);

impl MapBlobs {
    pub fn with(mut self, commit: &CommitSha, path: &str, text: &str) -> Self {
        self.0.insert(
            (commit.as_str().to_owned(), path.to_owned()),
            text.to_owned(),
        );
        self
    }
}

impl BlobSource for MapBlobs {
    fn read(&self, commit: &CommitSha, path: &RepoPath) -> Option<String> {
        self.0
            .get(&(commit.as_str().to_owned(), path.as_str().to_owned()))
            .cloned()
    }
}

fn info(title: &str, file: &str, start: u32, end: u32) -> NodeInfo {
    NodeInfo {
        title: title.to_owned(),
        file: path(file),
        start_line: start,
        end_line: end,
    }
}

/// Locations for the auth-bypass nodes.
pub fn locator() -> TableLocator {
    let mut t = BTreeMap::new();
    t.insert(
        key(UPDATE_USER),
        info(
            "AdminService.updateUser",
            "src/admin/admin.service.ts",
            20,
            60,
        ),
    );
    t.insert(
        key(CONTROLLER),
        info(
            "UserController.update",
            "src/users/user.controller.ts",
            5,
            25,
        ),
    );
    t.insert(
        key(PROVIDER),
        info("AuthProvider.authorize", "src/auth/auth.provider.ts", 3, 20),
    );
    t.insert(
        key(PERMISSION_CHECK),
        info(
            "PermissionService.check",
            "src/auth/permission.service.ts",
            2,
            12,
        ),
    );
    t.insert(
        key(SPEC),
        info("authorize denies", "src/auth/auth.service.spec.ts", 10, 24),
    );
    t.insert(
        key(DECOY),
        info(
            "ReportService.generate",
            "src/reports/report.service.ts",
            1,
            30,
        ),
    );
    t.insert(
        key(FORMAT_HEADER),
        info("authorizeHeader", "src/util/format.ts", 1, 9),
    );
    TableLocator(t)
}

/// Every fixture file at both commits, 200 numbered lines each.
pub fn blobs() -> MapBlobs {
    let files = [
        "src/auth/auth.service.ts",
        "src/admin/admin.service.ts",
        "src/users/user.controller.ts",
        "src/auth/auth.provider.ts",
        "src/auth/permission.service.ts",
        "src/auth/auth.service.spec.ts",
        "src/reports/report.service.ts",
        "src/util/format.ts",
    ];
    let mut b = MapBlobs::default();
    for f in files {
        b = b.with(&head(), f, &numbered(200));
        b = b.with(&base(), f, &numbered(200));
    }
    b
}

/// Assembly options with the reviewer's weights and resolved default budget.
pub fn options<'a>(
    blobs: &'a dyn BlobSource,
    reviewer: ReviewerType,
) -> context_engine::assemble::AssembleOptions<'a> {
    let limits = context_engine::budget::resolve(reviewer, 1.0, u32::MAX, u32::MAX).unwrap();
    let mut hints = BTreeMap::new();
    hints.insert(
        (key(UPDATE_USER), DiffSide::Head),
        context_engine::assemble::LineHints {
            changed_lines: vec![],
            seed_call_lines: vec![45],
        },
    );
    context_engine::assemble::AssembleOptions {
        cluster_id: "cluster-1".to_owned(),
        limits,
        weights: context_engine::ranking::Weights::for_reviewer(reviewer),
        blobs,
        hints,
        semantic: None,
        semantic_disabled: None,
        seed_summary: None,
        semantic_space: None,
        risk_signals: Vec::new(),
        change_summary: context_engine::model::ChangeSummary::default(),
        input_hashes: vec!["cm".to_owned(), "im".to_owned()],
        flags: Vec::new(),
    }
}

/// A numbered file: line `n` is `line n`.
pub fn numbered(lines: u32) -> String {
    (1..=lines)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}
