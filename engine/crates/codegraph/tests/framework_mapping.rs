//! CG-006 acceptance: framework facts become generic graph nodes and edges.
//!
//! The facts are hand-built, because `lang-typescript`'s NestJS/TypeORM/BullMQ adapters are not
//! this crate's to run and because a hand-built fact makes the *attribute contract* the thing
//! under test rather than the adapter.
//!
//! The last test greps this crate's own sources for framework identifiers, which is what keeps the
//! mapper reusable for another language.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::collections::BTreeSet;

use analysis_ir::framework::FrameworkFactKind;
use analysis_ir::symbol::AttrValue;
use codegraph::linker::symbol_table::FileSymbols;
use codegraph::{Edge, EdgeKind, NodeId, NodeKey, NodeKind, Provenance, ResolvedBy};
use review_core::symbol::SymbolKind;

use support::UnitBuilder;

/// The [`FileSymbols`] a set of units produces, which is all the mapper needs.
fn tables(units: &[std::sync::Arc<analysis_ir::unit::ParsedUnit>]) -> codegraph::SymbolTable {
    codegraph::SymbolTable::build(units)
}

fn file_of<'a>(table: &'a codegraph::SymbolTable, path: &str) -> &'a FileSymbols {
    table
        .get(&support::path(path))
        .expect("fixture declares the file")
}

fn output_of(
    units: &[std::sync::Arc<analysis_ir::unit::ParsedUnit>],
    path: &str,
) -> codegraph::FrameworkOutput {
    let table = tables(units);
    let file = file_of(&table, path);
    let facts = units
        .iter()
        .find(|unit| unit.file.as_str() == path)
        .map(|unit| unit.framework.clone())
        .unwrap_or_default();
    codegraph::FrameworkMapper::map_file(&facts, file, &[])
}

/// `output_of` for the default fixture path.
fn controller_output(
    units: &[std::sync::Arc<analysis_ir::unit::ParsedUnit>],
) -> codegraph::FrameworkOutput {
    output_of(units, "src/app.controller.ts")
}

#[test]
fn route_fact_creates_endpoint_handled_by_and_routes_to() {
    let mut unit = UnitBuilder::new("src/app.controller.ts");
    let controller = unit.symbol(SymbolKind::Class, "AppController", 2);
    unit.exported(controller);
    unit.child(SymbolKind::Method, "findAll", Some(controller), 4);
    unit.fact(
        FrameworkFactKind::Controller,
        vec![("symbol", AttrValue::Str("AppController".to_owned()))],
        2,
    );
    unit.fact(
        FrameworkFactKind::HttpRoute,
        vec![
            ("symbol", AttrValue::Str("findAll".to_owned())),
            ("method", AttrValue::Str("get".to_owned())),
            ("path", AttrValue::Str("/users/:id".to_owned())),
            ("controller", AttrValue::Str("AppController".to_owned())),
        ],
        4,
    );
    let units = vec![unit.build()];
    let out = controller_output(&units);

    let endpoint = NodeId::http("GET", "/users/{}").unwrap();
    assert!(
        out.nodes.iter().any(|node| node.id == endpoint),
        "an endpoint node is required:\n{}",
        dump(&out)
    );
    let controller_key = symbol_key(&units, "AppController");
    let handler_key = symbol_key(&units, "AppController.findAll");
    assert!(has(
        &out.edges,
        EdgeKind::HandledBy,
        endpoint.key(),
        handler_key
    ));
    assert!(has(
        &out.edges,
        EdgeKind::RoutesTo,
        endpoint.key(),
        controller_key
    ));
    assert!(out
        .refinements
        .contains(&(controller_key, NodeKind::Controller)));
    assert!(
        !out.refinements.contains(&(handler_key, NodeKind::Handler)),
        "a method stays a Method; only a Function is refined to Handler"
    );
    assert!(out.issues.is_empty(), "{:?}", out.issues);
}

#[test]
fn a_method_handler_stays_a_method() {
    let mut unit = UnitBuilder::new("src/app.controller.ts");
    let controller = unit.symbol(SymbolKind::Class, "AppController", 2);
    let handler = unit.child(SymbolKind::Method, "findOne", Some(controller), 4);
    unit.fact(
        FrameworkFactKind::HttpRoute,
        vec![
            ("symbol", AttrValue::Str("findOne".to_owned())),
            ("method", AttrValue::Str("GET".to_owned())),
            ("path", AttrValue::Str("/users/{}".to_owned())),
        ],
        4,
    );
    let units = vec![unit.build()];
    let out = controller_output(&units);
    assert!(
        !out.refinements.contains(&(
            symbol_key(&units, "AppController.findOne"),
            NodeKind::Handler
        )),
        "a method stays a Method: only a Function is refined to Handler"
    );
    assert!(has(
        &out.edges,
        EdgeKind::HandledBy,
        NodeId::http("GET", "/users/{}").unwrap().key(),
        symbol_key(&units, "AppController.findOne")
    ));
    let _ = handler;
}

#[test]
fn controller_level_guard_authorizes_all_its_endpoints() {
    let mut unit = UnitBuilder::new("src/app.controller.ts");
    let controller = unit.symbol(SymbolKind::Class, "AppController", 2);
    unit.child(SymbolKind::Method, "list", Some(controller), 4);
    unit.child(SymbolKind::Method, "create", Some(controller), 8);
    let guard = unit.symbol(SymbolKind::Class, "JwtGuard", 14);
    unit.exported(guard);
    unit.fact(
        FrameworkFactKind::Controller,
        vec![("symbol", AttrValue::Str("AppController".to_owned()))],
        2,
    );
    for (method, line) in [("list", 4), ("create", 8)] {
        unit.fact(
            FrameworkFactKind::HttpRoute,
            vec![
                ("symbol", AttrValue::Str(method.to_owned())),
                ("method", AttrValue::Str("GET".to_owned())),
                (
                    "path",
                    AttrValue::Str(format!(
                        "/{}",
                        if method == "list" { "users" } else { "orders" }
                    )),
                ),
                ("controller", AttrValue::Str("AppController".to_owned())),
            ],
            line,
        );
    }
    unit.fact(
        FrameworkFactKind::Middleware,
        vec![
            ("symbol", AttrValue::Str("JwtGuard".to_owned())),
            ("controller", AttrValue::Str("AppController".to_owned())),
        ],
        14,
    );
    let units = vec![unit.build()];
    let out = controller_output(&units);
    let guard_key = symbol_key(&units, "JwtGuard");
    let list = NodeId::http("GET", "/users").unwrap().key();
    let create = NodeId::http("GET", "/orders").unwrap().key();
    assert!(has(&out.edges, EdgeKind::Authorizes, guard_key, list));
    assert!(has(&out.edges, EdgeKind::Authorizes, guard_key, create));
    assert!(out.refinements.contains(&(guard_key, NodeKind::Middleware)));
    let edge = out
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Authorizes && e.target == list)
        .expect("a guard authorizes the route");
    assert_eq!(edge.provenance, Provenance::Framework);
    assert_eq!(edge.resolved_by, ResolvedBy::Framework);
    assert!(!edge.flags.contains(codegraph::EdgeFlags::GLOBAL_SCOPE));
}

#[test]
fn global_guard_applies_to_endpoints_added_later() {
    let mut unit = UnitBuilder::new("src/app.controller.ts");
    let guard = unit.symbol(SymbolKind::Class, "AppGuard", 2);
    unit.exported(guard);
    unit.fact(
        FrameworkFactKind::Middleware,
        vec![
            ("symbol", AttrValue::Str("AppGuard".to_owned())),
            ("global", AttrValue::Bool(true)),
        ],
        2,
    );
    let units = vec![unit.build()];
    let out = controller_output(&units);
    assert!(
        out.global_facts
            .iter()
            .any(|fact| matches!(fact, codegraph::GlobalFact::GlobalGuard { .. })),
        "a global guard is returned, not fanned out inside the file: {:?}",
        out.global_facts
    );

    // An endpoint added by a later file is still authorized.
    let endpoint = late_endpoint();
    let edges = codegraph::FrameworkMapper::apply_globals(&out.global_facts, &endpoint);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].kind, EdgeKind::Authorizes);
    assert!(edges[0].flags.contains(codegraph::EdgeFlags::GLOBAL_SCOPE));
    let _ = symbol_key(&units, "AppGuard");
}

#[test]
fn queue_producer_and_consumer_share_queue_node() {
    let mut unit = UnitBuilder::new("src/mail.ts");
    let producer = unit.symbol(SymbolKind::Method, "enqueue", 2);
    let consumer_class = unit.symbol(SymbolKind::Class, "MailProcessor", 4);
    let consumer = unit.child(SymbolKind::Method, "handle", Some(consumer_class), 6);
    unit.fact(
        FrameworkFactKind::QueueProducer,
        vec![
            ("symbol", AttrValue::Str("enqueue".to_owned())),
            ("queue", AttrValue::Str("email".to_owned())),
            ("job", AttrValue::Str("welcome".to_owned())),
        ],
        2,
    );
    unit.fact(
        FrameworkFactKind::QueueConsumer,
        vec![
            ("symbol", AttrValue::Str("MailProcessor".to_owned())),
            ("queue", AttrValue::Str("email".to_owned())),
            ("job", AttrValue::Str("welcome".to_owned())),
        ],
        4,
    );
    unit.fact(
        FrameworkFactKind::QueueJobHandler,
        vec![
            ("symbol", AttrValue::Str("handle".to_owned())),
            ("queue", AttrValue::Str("email".to_owned())),
            ("job", AttrValue::Str("reset".to_owned())),
        ],
        6,
    );
    let units = vec![unit.build()];
    let out = output_of(&units, "src/mail.ts");

    let queue = NodeId::queue("email").unwrap();
    let queue_nodes: Vec<&codegraph::SyntheticNode> =
        out.nodes.iter().filter(|node| node.id == queue).collect();
    assert_eq!(
        queue_nodes.len(),
        1,
        "producer and consumer share one queue node"
    );
    let jobs = queue_nodes[0]
        .attrs
        .extra
        .iter()
        .find(|(key, _)| key == "jobs")
        .map(|(_, value)| value.clone());
    assert_eq!(
        jobs.as_deref(),
        Some("reset,welcome"),
        "job names are sorted"
    );

    let producer_key = symbol_key(&units, "enqueue");
    let consumer_class_key = symbol_key(&units, "MailProcessor");
    let consumer_key = symbol_key(&units, "MailProcessor.handle");
    assert!(has(
        &out.edges,
        EdgeKind::ProducesJob,
        producer_key,
        queue.key()
    ));
    assert!(has(
        &out.edges,
        EdgeKind::ConsumesJob,
        consumer_class_key,
        queue.key()
    ));
    assert!(has(
        &out.edges,
        EdgeKind::ConsumesJob,
        consumer_key,
        queue.key()
    ));
    // `QueueProducer` is not a refinement source kind in CG-001's contract: the producer stays a
    // method and is identified by its `PRODUCES_JOB` edge.
    assert!(
        !out.refinements
            .iter()
            .any(|(key, kind)| *key == producer_key && *kind == NodeKind::QueueProducer),
        "a producer method keeps its kind"
    );
    assert!(out
        .refinements
        .contains(&(consumer_class_key, NodeKind::QueueConsumer)));
    assert!(out
        .refinements
        .contains(&(consumer_key, NodeKind::JobHandler)));
    let _ = (producer, consumer);
}

#[test]
fn entity_and_db_access_create_table_edges_with_ops() {
    let mut unit = UnitBuilder::new("src/user.entity.ts");
    let entity = unit.symbol(SymbolKind::Class, "UserEntity", 2);
    unit.exported(entity);
    let repository = unit.symbol(SymbolKind::Method, "save", 6);
    let finder = unit.symbol(SymbolKind::Method, "find", 10);
    unit.fact(
        FrameworkFactKind::OrmEntity,
        vec![
            ("symbol", AttrValue::Str("UserEntity".to_owned())),
            ("table", AttrValue::Str("Users".to_owned())),
            ("schema", AttrValue::Str("App".to_owned())),
        ],
        2,
    );
    for (symbol, op, line) in [("save", "write", 6), ("find", "read", 10)] {
        unit.fact(
            FrameworkFactKind::OrmAccess,
            vec![
                ("symbol", AttrValue::Str(symbol.to_owned())),
                ("entity", AttrValue::Str("Users".to_owned())),
                ("op", AttrValue::Str(op.to_owned())),
            ],
            line,
        );
    }
    let units = vec![unit.build()];
    let out = output_of(&units, "src/user.entity.ts");

    let table = NodeId::table(Some("App"), "Users").unwrap();
    assert!(out.nodes.iter().any(|node| node.id == table));
    let entity_key = symbol_key(&units, "UserEntity");
    assert!(has(
        &out.edges,
        EdgeKind::References,
        entity_key,
        table.key()
    ));
    let mapping_edge = out
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::References && e.target == table.key())
        .expect("the entity maps onto its table");
    assert!(mapping_edge
        .flags
        .contains(codegraph::EdgeFlags::MAPS_TABLE));
    assert!(has(
        &out.edges,
        EdgeKind::WritesTable,
        symbol_key(&units, "save"),
        table.key()
    ));
    assert!(has(
        &out.edges,
        EdgeKind::ReadsTable,
        symbol_key(&units, "find"),
        table.key()
    ));
    assert!(out
        .refinements
        .contains(&(entity_key, NodeKind::DatabaseEntity)));
    let _ = (repository, finder);
}

#[test]
fn env_read_creates_env_node_without_value() {
    let mut unit = UnitBuilder::new("src/config.ts");
    unit.symbol(SymbolKind::Function, "readConfig", 2);
    unit.fact(
        FrameworkFactKind::ConfigRead,
        vec![
            ("symbol", AttrValue::Str("readConfig".to_owned())),
            ("name", AttrValue::Str("DATABASE_URL".to_owned())),
        ],
        2,
    );
    let units = vec![unit.build()];
    let out = output_of(&units, "src/config.ts");
    let env = NodeId::env("DATABASE_URL").unwrap();
    let node = out
        .nodes
        .iter()
        .find(|node| node.id == env)
        .expect("env node");
    assert_eq!(node.kind, NodeKind::EnvironmentVariable);
    assert_eq!(node.name, "DATABASE_URL");
    // The name is the whole payload: a secret in the variable never reaches the graph.
    assert!(
        !support::dump_output(&out).contains("://"),
        "no value may appear in the node: {:?}",
        node
    );
    assert!(has(
        &out.edges,
        EdgeKind::ReadsConfig,
        symbol_key(&units, "readConfig"),
        env.key()
    ));
}

#[test]
fn test_case_tests_edges_derive_confidence_from_calls() {
    let mut unit = UnitBuilder::new("src/user.service.spec.ts");
    let subject = unit.symbol(SymbolKind::Method, "findOne", 4);
    unit.exported(subject);
    let test_case = unit.symbol(SymbolKind::Function, "finds one user", 8);
    unit.fact(
        FrameworkFactKind::TestCase,
        vec![
            ("symbol", AttrValue::Str("finds one user".to_owned())),
            ("suite_path", AttrValue::Str("UserService".to_owned())),
            ("name", AttrValue::Str("finds one user".to_owned())),
            (
                "range",
                AttrValue::List(vec![
                    AttrValue::Int(8),
                    AttrValue::Int(0),
                    AttrValue::Int(12),
                    AttrValue::Int(0),
                ]),
            ),
        ],
        8,
    );

    // The call the test body makes, resolved with a weak confidence so the derivation is visible.
    let units = vec![unit.build()];
    let subject_key = symbol_key(&units, "findOne");
    let call = Edge::new(
        EdgeKind::Calls,
        symbol_key(&units, "finds one user"),
        subject_key,
        codegraph::confidence_of(ResolvedBy::NameAmbiguous),
        ResolvedBy::NameAmbiguous,
        Provenance::Linker,
    )
    .with_location(codegraph::Location::new(
        support::path("src/user.service.spec.ts"),
        9,
        4,
    ));

    let table = tables(&units);
    let file = file_of(&table, "src/user.service.spec.ts");
    let out = codegraph::FrameworkMapper::map_file(&units[0].framework, file, &[call]);
    let case_id = NodeId::test(
        &support::path("src/user.service.spec.ts"),
        "UserService",
        "finds one user",
    )
    .unwrap();
    let case = case_id.key();
    let edge = out
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Tests && e.source == case)
        .expect("the test points at what it calls");
    assert_eq!(edge.target, subject_key);
    assert_eq!(
        edge.confidence,
        codegraph::derived(
            ResolvedBy::Framework,
            codegraph::confidence_of(ResolvedBy::NameAmbiguous)
        ),
        "a derived edge is the weaker of the rule and its input"
    );
    assert!(edge.confidence < codegraph::confidence_of(ResolvedBy::Framework));
    let _ = test_case;
}

#[test]
fn refinement_keeps_symbol_key() {
    let mut unit = UnitBuilder::new("src/app.controller.ts");
    let controller = unit.symbol(SymbolKind::Class, "AppController", 2);
    unit.fact(
        FrameworkFactKind::Controller,
        vec![("symbol", AttrValue::Str("AppController".to_owned()))],
        2,
    );
    let units = vec![unit.build()];
    let out = controller_output(&units);
    let key = symbol_key(&units, "AppController");
    assert!(out.refinements.contains(&(key, NodeKind::Controller)));
    // The refinement is a `(key, kind)` pair: the key is the canonical id's hash and therefore
    // unchanged by the refinement.
    let refined = out
        .refinements
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .unwrap();
    assert_eq!(
        refined.0,
        NodeId::from_canonical("ts:src/app.controller#AppController/class").key()
    );
    assert_eq!(controller.0, 1);
}

#[test]
fn invalid_fact_attrs_produce_issue_not_error() {
    let mut unit = UnitBuilder::new("src/broken.ts");
    let handler = unit.symbol(SymbolKind::Method, "list", 2);
    // No `method`, no `path`.
    unit.fact(
        FrameworkFactKind::HttpRoute,
        vec![("symbol", AttrValue::Str("list".to_owned()))],
        2,
    );
    // `method` is not a string.
    unit.fact(
        FrameworkFactKind::HttpRoute,
        vec![
            ("symbol", AttrValue::Str("list".to_owned())),
            ("method", AttrValue::Int(1)),
            ("path", AttrValue::Str("/x".to_owned())),
        ],
        2,
    );
    // The symbol does not exist.
    unit.fact(
        FrameworkFactKind::HttpRoute,
        vec![
            ("symbol", AttrValue::Str("ghost".to_owned())),
            ("method", AttrValue::Str("GET".to_owned())),
            ("path", AttrValue::Str("/ghost".to_owned())),
        ],
        2,
    );
    // A `db_access` whose entity no entity fact declares.
    unit.fact(
        FrameworkFactKind::OrmAccess,
        vec![
            ("symbol", AttrValue::Str("list".to_owned())),
            ("entity", AttrValue::Str("ghost_table".to_owned())),
            ("op", AttrValue::Str("read".to_owned())),
        ],
        2,
    );
    // A `db_access` with an unknown op.
    unit.fact(
        FrameworkFactKind::OrmAccess,
        vec![
            ("symbol", AttrValue::Str("list".to_owned())),
            ("entity", AttrValue::Str("t".to_owned())),
            ("op", AttrValue::Str("upsert".to_owned())),
        ],
        2,
    );
    let units = vec![unit.build()];
    let out = output_of(&units, "src/broken.ts");

    let codes: BTreeSet<(&str, &str)> = out
        .issues
        .iter()
        .map(|issue| (issue.category.as_str(), issue.code.as_str()))
        .collect();
    assert!(codes.contains(&("route", "missing_attribute")), "{codes:?}");
    assert!(codes.contains(&("route", "invalid_attribute")), "{codes:?}");
    assert!(codes.contains(&("route", "unknown_symbol")), "{codes:?}");
    assert!(
        codes.contains(&("db_access", "unresolved_entity")),
        "{codes:?}"
    );
    assert!(
        codes.contains(&("db_access", "invalid_attribute")),
        "{codes:?}"
    );
    // Nothing was produced from the broken facts.
    assert!(out.nodes.is_empty(), "{:?}", out.nodes);
    assert!(out.edges.is_empty(), "{:?}", out.edges);
    let _ = handler;
}

#[test]
fn codegraph_contains_no_framework_identifiers() {
    // The mapper must stay language-neutral: no adapter's vocabulary may leak into it.
    let banned = [
        "@Controller",
        "@Injectable",
        "UseGuards",
        "@Entity",
        "@Column",
        "BullModule",
        "@Processor",
        "@nestjs",
        "typeorm",
        "bullmq",
        "jest",
    ];
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    collect_sources(&root, &mut files);
    assert!(!files.is_empty(), "the source tree must not be empty");
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for needle in banned {
            assert!(
                !text.contains(needle),
                "{} mentions {needle}",
                file.display()
            );
        }
    }
}

/// Every `.rs` file under `dir`, recursively.
fn collect_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The key of a symbol the fixture declares, by its qualified name.
fn symbol_key(units: &[std::sync::Arc<analysis_ir::unit::ParsedUnit>], qualified: &str) -> NodeKey {
    let unit = &units[0];
    let symbol = unit
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name.join(".") == qualified)
        .unwrap_or_else(|| panic!("{qualified} is not declared"));
    match symbol.local_id.0 {
        0 => NodeId::file(&unit.file).key(),
        _ => NodeId::from_canonical(format!(
            "ts:{}#{qualified}/{}",
            unit.module_path.as_str(),
            symbol.kind.as_id_str()
        ))
        .key(),
    }
}

fn has(edges: &[Edge], kind: EdgeKind, source: NodeKey, target: NodeKey) -> bool {
    edges
        .iter()
        .any(|edge| edge.kind == kind && edge.source == source && edge.target == target)
}

/// One endpoint, as `apply_globals` takes it.
fn late_endpoint() -> Vec<(NodeKey, codegraph::SyntheticNode)> {
    let id = NodeId::http("GET", "/late").unwrap();
    vec![(
        id.key(),
        codegraph::graph::NodeInput::new(id, NodeKind::ApiEndpoint, "GET /late"),
    )]
}

/// A readable dump, so a failed assertion shows what the mapper produced.
fn dump(out: &codegraph::FrameworkOutput) -> String {
    support::dump_output(out)
}
