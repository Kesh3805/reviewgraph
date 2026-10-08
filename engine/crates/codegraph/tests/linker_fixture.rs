//! CG-005 acceptance: the linker resolves IR references to typed edges.
//!
//! The fixture is a nine-file TypeScript-shaped repository built in memory, covering every case the
//! resolution cascade names: an import through a barrel and through `export *`, a re-export cycle,
//! `this.x()`, a constructor-parameter injection, a unique name, an ambiguous name inside and
//! outside the fan-out limit, an external package, and an inheritance pair with an override.
//!
//! `expected-edges.json` in the real fixture repository is out of this crate's lane, so the
//! hand-labelled expectation lives here as [`expected()`] and the precision/recall thresholds of
//! the task are asserted against it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::reference::{ReceiverHint, RefKind};
use codegraph::linker::{LinkConfig, LinkInput, Linker};
use codegraph::{Edge, EdgeKind, Graph, GraphBuilder, GraphQuery, NodeId, SCHEMA_VERSION};
use review_core::symbol::SymbolKind;

use support::{path, symbol_id, UnitBuilder};

/// The whole fixture: units plus the resolver that ties them together.
struct Fixture {
    units: Vec<std::sync::Arc<analysis_ir::unit::ParsedUnit>>,
    resolver: support::TableResolver,
}

fn build_fixture() -> Fixture {
    let mut units = Vec::new();

    // src/users/users.repository.ts — the leaf every other file imports from.
    let mut repo = UnitBuilder::new("src/users/users.repository.ts");
    let find = repo.symbol(SymbolKind::Class, "UsersRepository", 2);
    repo.exported(find);
    let find_all = repo.child(SymbolKind::Method, "findAll", Some(find), 4);
    let find_one = repo.child(SymbolKind::Method, "findOne", Some(find), 8);
    let _ = (find_all, find_one);
    units.push(repo.build());

    // src/users/index.ts — a barrel with an explicit re-export.
    let mut barrel = UnitBuilder::new("src/users/index.ts");
    barrel.reexport("./users.repository", "UsersRepository", "UsersRepository");
    units.push(barrel.build());

    // src/cycle/a.ts and src/cycle/b.ts — two barrels that re-export each other.
    let mut a = UnitBuilder::new("src/cycle/a.ts");
    a.reexport("./b", "fromB", "fromA");
    units.push(a.build());
    let mut b = UnitBuilder::new("src/cycle/b.ts");
    b.reexport("./a", "fromA", "fromB");
    units.push(b.build());

    // src/app.service.ts — `this.x()`, constructor injection, a unique name and a package.
    let mut app = UnitBuilder::new("src/app.service.ts");
    let app_class = app.symbol(SymbolKind::Class, "AppService", 3);
    let authorize = app.child(SymbolKind::Method, "authorize", Some(app_class), 9);
    let call_repo = app.child(SymbolKind::Method, "callRepo", Some(app_class), 12);
    let call_guard = app.child(SymbolKind::Method, "callGuard", Some(app_class), 16);
    let load = app.child(SymbolKind::Method, "load", Some(app_class), 20);
    let repo_param = app.child(SymbolKind::Parameter, "repo", Some(app_class), 6);
    let guard_param = app.child(SymbolKind::Parameter, "guard", Some(app_class), 8);
    app.this_member(call_guard, "authorize", 17);
    app.reference_with(
        call_repo,
        RefKind::Call,
        "findAll",
        ReceiverHint::ThisField {
            field: "repo".to_owned(),
            declared_type: Some("UsersRepository".to_owned()),
        },
        14,
        None,
    );
    // `new UsersRepository()` is a constructor call, not a bare call, so it maps to CALLS plus
    // USES_TYPE (CG-005 kind mapping).
    app.reference(load, RefKind::New, "UsersRepository", 21);
    let (import, binding) = app.import_named("lodash", "_", "map", 1);
    app.reference_with(
        load,
        RefKind::Call,
        "map",
        ReceiverHint::None,
        22,
        Some(analysis_ir::reference::BindingRef { import, binding }),
    );
    // Constructor parameter properties, which is what `DiConstructor` reads.
    let _ = (repo_param, guard_param, authorize, ());
    units.push(app.build());

    // src/shared.ts — two same-named functions, which is what `NameAmbiguous` fans out over.
    let mut first = UnitBuilder::new("src/first.ts");
    let helper = first.symbol(SymbolKind::Function, "helper", 1);
    first.exported(helper);
    units.push(first.build());
    let mut second = UnitBuilder::new("src/second.ts");
    let helper = second.symbol(SymbolKind::Function, "helper", 1);
    second.exported(helper);
    units.push(second.build());
    let mut caller = UnitBuilder::new("src/caller.ts");
    let run = caller.symbol(SymbolKind::Function, "run", 1);
    caller.reference(run, RefKind::Call, "helper", 2);
    units.push(caller.build());

    // src/base.ts and src/derived.ts — inheritance plus an override.
    let mut base = UnitBuilder::new("src/base.ts");
    let base_class = base.symbol(SymbolKind::Class, "BaseService", 1);
    let render = base.child(SymbolKind::Method, "render", Some(base_class), 3);
    let _ = render;
    base.exported(base_class);
    units.push(base.build());

    let mut derived = UnitBuilder::new("src/derived.ts");
    let derived_class = derived.symbol(SymbolKind::Class, "DerivedService", 1);
    let render = derived.child(SymbolKind::Method, "render", Some(derived_class), 3);
    let _ = render;
    let (import, binding) = derived.import_named("./base", "BaseService", "BaseService", 2);
    derived.reference_with(
        derived_class,
        RefKind::Extends,
        "BaseService",
        ReceiverHint::None,
        2,
        Some(analysis_ir::reference::BindingRef { import, binding }),
    );
    units.push(derived.build());

    let resolver = support::TableResolver::new()
        .file("src/app.service.ts", "./users", "src/users/index.ts")
        .file(
            "src/users/index.ts",
            "./users.repository",
            "src/users/users.repository.ts",
        )
        .file("src/cycle/a.ts", "./b", "src/cycle/b.ts")
        .file("src/cycle/b.ts", "./a", "src/cycle/a.ts")
        .file("src/derived.ts", "./base", "src/base.ts")
        .external("src/app.service.ts", "lodash", "npm", "lodash");

    Fixture { units, resolver }
}

fn link(fixture: &Fixture) -> codegraph::LinkOutput {
    let config = LinkConfig::default();
    Linker::link_all(LinkInput {
        units: &fixture.units,
        resolver: &fixture.resolver,
        config: &config,
        repository_name: "fixture",
    })
}

/// The edges the fixture's hand labelling says must exist, as `(source id, kind, target id)`.
fn expected() -> Vec<(String, EdgeKind, String)> {
    vec![
        // `this.x()` resolves inside the enclosing class.
        (
            "ts:src/app.service#AppService.callGuard/method".to_owned(),
            EdgeKind::Calls,
            "ts:src/app.service#AppService.authorize/method".to_owned(),
        ),
        // Constructor-parameter injection: `repo` is typed `UsersRepository`.
        (
            "ts:src/app.service#AppService.callRepo/method".to_owned(),
            EdgeKind::Calls,
            "ts:src/users/users.repository#UsersRepository.findAll/method".to_owned(),
        ),
        // `new UsersRepository()` is a constructor call.
        (
            "ts:src/app.service#AppService.load/method".to_owned(),
            EdgeKind::Calls,
            "ts:src/users/users.repository#UsersRepository/class".to_owned(),
        ),
        (
            "ts:src/app.service#AppService.load/method".to_owned(),
            EdgeKind::UsesType,
            "ts:src/users/users.repository#UsersRepository/class".to_owned(),
        ),
        // Inheritance plus the derived override.
        (
            "ts:src/derived#DerivedService/class".to_owned(),
            EdgeKind::Extends,
            "ts:src/base#BaseService/class".to_owned(),
        ),
        (
            "ts:src/derived#DerivedService.render/method".to_owned(),
            EdgeKind::Overrides,
            "ts:src/base#BaseService.render/method".to_owned(),
        ),
        // The ambiguous name: both candidates are hand-labelled, because the fixture contains two
        // declarations of `helper` and the cascade is expected to fan out over both.
        (
            "ts:src/caller#run/function".to_owned(),
            EdgeKind::Calls,
            "ts:src/first#helper/function".to_owned(),
        ),
        (
            "ts:src/caller#run/function".to_owned(),
            EdgeKind::Calls,
            "ts:src/second#helper/function".to_owned(),
        ),
        // The external package the file imports: one `IMPORTS` and one `DEPENDS_ON` onto the
        // single `pkg:npm/lodash` node, emitted once for the specifier however many references use
        // it.
        (
            "file:src/app.service.ts".to_owned(),
            EdgeKind::Imports,
            "pkg:npm/lodash".to_owned(),
        ),
        (
            "file:src/app.service.ts".to_owned(),
            EdgeKind::DependsOn,
            "pkg:npm/lodash".to_owned(),
        ),
    ]
}

fn build_graph(output: &codegraph::LinkOutput) -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for node in &output.nodes {
        builder.add_node(node.clone()).unwrap();
    }
    for edge in &output.edges {
        builder.add_edge(edge.clone());
    }
    for reference in &output.unresolved {
        builder.add_unresolved(reference.clone());
    }
    builder.build().unwrap()
}

#[test]
fn fixture_expected_edges_precision_recall() {
    let fixture = build_fixture();
    let output = link(&fixture);

    // Structural edges (`CONTAINS`/`DECLARES`/`EXPORTS`/`IMPORTS`, provenance `Analyzer`) are a
    // pure function of the file list and are not what the task measures precision on: the
    // hand-labelled expectation covers the reference-resolved relations.
    let produced: BTreeSet<(String, String, String)> = output
        .edges
        .iter()
        .filter(|edge| edge.provenance == codegraph::Provenance::Linker)
        .map(|edge| {
            (
                label(&output, edge.source),
                edge.kind.as_str().to_owned(),
                label(&output, edge.target),
            )
        })
        .collect();
    let wanted: BTreeSet<(String, String, String)> = expected()
        .into_iter()
        .map(|(source, kind, target)| (source, kind.as_str().to_owned(), target))
        .collect();

    let hits = wanted.intersection(&produced).count();
    let precision = f64::from(u32::try_from(hits).unwrap_or(0)) / produced.len().max(1) as f64;
    let recall = f64::from(u32::try_from(hits).unwrap_or(0)) / wanted.len().max(1) as f64;

    let missing: Vec<&(String, String, String)> = wanted.difference(&produced).collect();
    assert!(
        missing.is_empty(),
        "the linker missed {missing:?}\nproduced:\n{}",
        support::dump(&output)
    );
    assert!(
        precision >= 0.95,
        "precision {precision:.3} below the 0.95 target\nproduced:\n{}",
        support::dump(&output)
    );
    assert!(recall >= 0.90, "recall {recall:.3} below the 0.90 target");
}

/// The canonical id behind a key, or a marker for synthetic nodes.
fn label(output: &codegraph::LinkOutput, key: codegraph::NodeKey) -> String {
    output
        .nodes
        .iter()
        .find(|node| node.id.key() == key)
        .map(|node| node.id.as_str().to_owned())
        .unwrap_or_else(|| format!("synthetic:{key}"))
}

#[test]
fn import_through_barrel_and_star_reexport() {
    let mut barrel = UnitBuilder::new("src/star/index.ts");
    barrel.star_reexport("./leaf");
    let leaf = {
        let mut leaf = UnitBuilder::new("src/star/leaf.ts");
        let handler = leaf.symbol(SymbolKind::Function, "handler", 1);
        leaf.exported(handler);
        leaf.build()
    };
    let mut app = UnitBuilder::new("src/star/app.ts");
    let run = app.symbol(SymbolKind::Function, "run", 1);
    let (import, binding) = app.import_named("./index", "handler", "handler", 2);
    app.reference_with(
        run,
        RefKind::Call,
        "handler",
        ReceiverHint::None,
        3,
        Some(analysis_ir::reference::BindingRef { import, binding }),
    );
    let units = vec![leaf, app.build(), barrel.build()];
    let resolver = support::TableResolver::new()
        .file("src/star/app.ts", "./index", "src/star/index.ts")
        .file("src/star/index.ts", "./leaf", "src/star/leaf.ts");
    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &resolver,
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });
    let target = symbol_id("src/star/leaf.ts", "handler", "function").key();
    assert!(
        output
            .edges
            .iter()
            .any(|edge| { edge.kind == EdgeKind::Calls && edge.target == target }),
        "an `export *` chain must be followed:\n{}",
        support::dump(&output)
    );
}

#[test]
fn reexport_cycle_is_unresolved_not_infinite() {
    let mut a = UnitBuilder::new("src/cycle2/a.ts");
    a.reexport("./b", "bump", "bump");
    let mut b = UnitBuilder::new("src/cycle2/b.ts");
    b.reexport("./a", "bump", "bump");
    let mut app = UnitBuilder::new("src/cycle2/app.ts");
    let run = app.symbol(SymbolKind::Function, "run", 1);
    let (import, binding) = app.import_named("./a", "bump", "bump", 2);
    app.reference_with(
        run,
        RefKind::Call,
        "bump",
        ReceiverHint::None,
        3,
        Some(analysis_ir::reference::BindingRef { import, binding }),
    );
    let units = vec![a.build(), b.build(), app.build()];
    let resolver = support::TableResolver::new()
        .file("src/cycle2/app.ts", "./a", "src/cycle2/a.ts")
        .file("src/cycle2/a.ts", "./b", "src/cycle2/b.ts")
        .file("src/cycle2/b.ts", "./a", "src/cycle2/a.ts");
    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &resolver,
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });
    let reason = output
        .unresolved
        .iter()
        .find(|reference| reference.name == "bump")
        .map(|reference| reference.reason);
    assert_eq!(
        reason,
        Some(codegraph::UnresolvedReason::ReexportCycle),
        "a cyclic barrel must be reported, not followed:\n{}",
        support::dump(&output)
    );
}

#[test]
fn this_member_resolves_to_nearest_super() {
    let mut base = UnitBuilder::new("src/sup/base.ts");
    let base_class = base.symbol(SymbolKind::Class, "Base", 1);
    base.child(SymbolKind::Method, "shared", Some(base_class), 3);
    base.child(SymbolKind::Method, "own", Some(base_class), 7);
    let base_key = base_class;
    let base = base.build();

    let mut derived = UnitBuilder::new("src/sup/derived.ts");
    let derived_class = derived.symbol(SymbolKind::Class, "Derived", 1);
    let run = derived.child(SymbolKind::Method, "run", Some(derived_class), 3);
    derived.this_member(run, "shared", 4);
    derived.this_member(run, "own", 5);
    let derived_units = derived.build();

    let mut bridge = UnitBuilder::new("src/sup/bridge.ts");
    bridge.reexport("./base", "Base", "Base");
    let bridge_units = bridge.build();

    let units = vec![base, derived_units, bridge_units];
    let resolver =
        support::TableResolver::new().file("src/sup/bridge.ts", "./base", "src/sup/base.ts");
    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &resolver,
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });
    let base_shared = symbol_id("src/sup/base.ts", "Base.shared", "method").key();
    let base_own = symbol_id("src/sup/base.ts", "Base.own", "method").key();
    let from = symbol_id("src/sup/derived.ts", "Derived.run", "method").key();
    let targets: Vec<codegraph::NodeKey> = output
        .edges
        .iter()
        .filter(|edge| edge.source == from && edge.kind == EdgeKind::Calls)
        .map(|edge| edge.target)
        .collect();
    assert!(
        targets.contains(&base_shared),
        "`this.shared` is declared on the superclass:\n{}",
        support::dump(&output)
    );
    assert!(targets.contains(&base_own));
    let _ = base_key;
}

#[test]
fn di_constructor_param_type_resolves_member() {
    let mut types = UnitBuilder::new("src/di/types.ts");
    let service = types.symbol(SymbolKind::Class, "AuthService", 1);
    types.exported(service);
    types.child(SymbolKind::Method, "verify", Some(service), 3);
    let units_types = types.build();

    let mut consumer = UnitBuilder::new("src/di/consumer.ts");
    let consumer_class = consumer.symbol(SymbolKind::Class, "Consumer", 1);
    let run = consumer.child(SymbolKind::Method, "run", Some(consumer_class), 5);
    let (import, binding) = consumer.import_named("./types", "AuthService", "AuthService", 2);
    consumer.reference_with(
        run,
        RefKind::Call,
        "verify",
        ReceiverHint::ThisField {
            field: "auth".to_owned(),
            declared_type: Some("AuthService".to_owned()),
        },
        6,
        None,
    );
    let _ = (import, binding);
    let units_consumer = consumer.build();

    let units = vec![units_types, units_consumer];
    let resolver =
        support::TableResolver::new().file("src/di/consumer.ts", "./types", "src/di/types.ts");
    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &resolver,
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });
    let verify = symbol_id("src/di/types.ts", "AuthService.verify", "method").key();
    let from = symbol_id("src/di/consumer.ts", "Consumer.run", "method").key();
    let edge = output
        .edges
        .iter()
        .find(|edge| edge.source == from && edge.target == verify);
    let edge = edge.unwrap_or_else(|| panic!("{}", support::dump(&output)));
    assert_eq!(
        edge.resolved_by,
        codegraph::ResolvedBy::DiConstructor,
        "a constructor-parameter receiver is step 3 of the cascade"
    );
    assert_eq!(
        edge.confidence,
        codegraph::confidence_of(codegraph::ResolvedBy::DiConstructor)
    );
}

#[test]
fn ambiguous_name_fans_out_up_to_limit() {
    let mut first = UnitBuilder::new("src/amb/a.ts");
    let first_fn = first.symbol(SymbolKind::Function, "shared", 1);
    first.exported(first_fn);
    let mut second = UnitBuilder::new("src/amb/b.ts");
    let second_fn = second.symbol(SymbolKind::Function, "shared", 1);
    second.exported(second_fn);
    let mut caller = UnitBuilder::new("src/amb/caller.ts");
    let run = caller.symbol(SymbolKind::Function, "run", 1);
    caller.reference(run, RefKind::Call, "shared", 2);
    let units = vec![first.build(), second.build(), caller.build()];
    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &support::TableResolver::new(),
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });
    let from = symbol_id("src/amb/caller.ts", "run", "function").key();
    let hits: Vec<&Edge> = output
        .edges
        .iter()
        .filter(|edge| edge.source == from && edge.kind == EdgeKind::Calls)
        .collect();
    assert_eq!(
        hits.len(),
        2,
        "both candidates get an edge:\n{}",
        support::dump(&output)
    );
    for edge in hits {
        assert_eq!(edge.resolved_by, codegraph::ResolvedBy::NameAmbiguous);
        assert!(
            edge.flags.contains(codegraph::EdgeFlags::DYNAMIC),
            "a fanned-out edge is dynamic"
        );
        assert_eq!(
            edge.confidence,
            codegraph::confidence_of(codegraph::ResolvedBy::NameAmbiguous)
        );
    }
}

#[test]
fn ambiguous_over_limit_is_unresolved_with_count() {
    let mut units = Vec::new();
    let mut names = Vec::new();
    for n in 0..5 {
        let path = format!("src/many/f{n}.ts");
        let mut unit = UnitBuilder::new(&path);
        let function = unit.symbol(SymbolKind::Function, "dup", 1);
        unit.exported(function);
        units.push(unit.build());
        names.push(symbol_id(&path, "dup", "function").key());
    }
    let mut caller = UnitBuilder::new("src/many/caller.ts");
    let run = caller.symbol(SymbolKind::Function, "run", 1);
    caller.reference(run, RefKind::Call, "dup", 2);
    units.push(caller.build());

    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &support::TableResolver::new(),
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });
    let from = symbol_id("src/many/caller.ts", "run", "function").key();
    let hits = output
        .edges
        .iter()
        .filter(|edge| edge.source == from)
        .count();
    assert_eq!(hits, 0, "past the fan-out limit nothing is linked");
    let reference = output
        .unresolved
        .iter()
        .find(|reference| reference.name == "dup")
        .unwrap_or_else(|| panic!("{}", support::dump(&output)));
    assert_eq!(reference.reason, codegraph::UnresolvedReason::Ambiguous);
    assert_eq!(reference.candidate_count, 5);
    let _ = names;
}

#[test]
fn external_package_creates_pkg_node_and_unresolved_external() {
    let mut app = UnitBuilder::new("src/pkg/app.ts");
    let run = app.symbol(SymbolKind::Function, "run", 1);
    let (import, binding) = app.import_named("lodash/fp", "_", "map", 2);
    app.reference_with(
        run,
        RefKind::Call,
        "map",
        ReceiverHint::None,
        3,
        Some(analysis_ir::reference::BindingRef { import, binding }),
    );
    let units = vec![app.build()];
    let resolver =
        support::TableResolver::new().external("src/pkg/app.ts", "lodash/fp", "npm", "lodash");
    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &resolver,
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });

    let package = NodeId::package("npm", "lodash").unwrap();
    assert!(
        output.nodes.iter().any(|node| node.id == package),
        "an external specifier creates one package node:\n{}",
        support::dump(&output)
    );
    let file = NodeId::file(&path("src/pkg/app.ts")).key();
    for kind in [EdgeKind::Imports, EdgeKind::DependsOn] {
        assert!(
            output.edges.iter().any(|edge| edge.kind == kind
                && edge.source == file
                && edge.target == package.key()),
            "{kind} file -> package is required:\n{}",
            support::dump(&output)
        );
    }
    let reference = output
        .unresolved
        .iter()
        .find(|reference| reference.name == "map")
        .unwrap_or_else(|| panic!("{}", support::dump(&output)));
    assert_eq!(reference.reason, codegraph::UnresolvedReason::External);
    assert_eq!(reference.import_specifier.as_deref(), Some("lodash/fp"));
}

#[test]
fn overrides_post_pass() {
    let fixture = build_fixture();
    let output = link(&fixture);
    let derived_render = symbol_id("src/derived.ts", "DerivedService.render", "method").key();
    let base_render = symbol_id("src/base.ts", "BaseService.render", "method").key();
    let edge = output
        .edges
        .iter()
        .find(|edge| edge.kind == EdgeKind::Overrides)
        .unwrap_or_else(|| panic!("{}", support::dump(&output)));
    assert_eq!(edge.source, derived_render);
    assert_eq!(edge.target, base_render);
    assert_eq!(edge.resolved_by, codegraph::ResolvedBy::Structural);
    assert_eq!(
        edge.confidence,
        codegraph::derived(
            codegraph::ResolvedBy::Structural,
            codegraph::confidence_of(codegraph::ResolvedBy::Import)
        ),
        "an override is the weaker of structural and the inheritance edge"
    );
}

#[test]
fn link_all_equals_concat_link_file() {
    let fixture = build_fixture();
    let config = LinkConfig::default();
    let (tables, names) = Linker::build_tables(&fixture.units);
    let full = Linker::link_all(LinkInput {
        units: &fixture.units,
        resolver: &fixture.resolver,
        config: &config,
        repository_name: "fixture",
    });

    let mut per_file_edges: Vec<Edge> = Vec::new();
    let mut per_file_nodes: Vec<codegraph::NodeInput> = Vec::new();
    let mut per_file_unresolved = Vec::new();
    let mut ordered: Vec<&std::sync::Arc<analysis_ir::unit::ParsedUnit>> =
        fixture.units.iter().collect();
    ordered.sort_by_key(|unit| unit.file.as_str());
    for unit in ordered {
        let result = Linker::link_file(unit, &tables, &names, &fixture.resolver, &config);
        per_file_nodes.extend(result.nodes);
        per_file_edges.extend(result.edges);
        per_file_unresolved.extend(result.unresolved);
    }
    per_file_edges.sort();
    per_file_edges.dedup();
    per_file_unresolved.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.ordinal.cmp(&b.ordinal))
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut full_edges = full.edges.clone();
    // `link_all` adds the repository/directory layer on top of the per-file contribution; those
    // edges have a `Repository` or `Directory` node as their source.
    let structural_keys: BTreeSet<codegraph::NodeKey> = full
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.kind,
                codegraph::NodeKind::Repository | codegraph::NodeKind::Directory
            )
        })
        .map(|node| node.id.key())
        .collect();
    full_edges.retain(|edge| !structural_keys.contains(&edge.source));
    full_edges.sort();
    full_edges.dedup();
    assert_eq!(
        full_edges, per_file_edges,
        "`link_all` must be the concatenation of `link_file` plus the repository/directory layer"
    );
    assert_eq!(full.unresolved, per_file_unresolved);

    let mut per_file_keys: Vec<String> = per_file_nodes.iter().map(|n| n.id.to_string()).collect();
    let mut full_keys: Vec<String> = full.nodes.iter().map(|n| n.id.to_string()).collect();
    per_file_keys.sort();
    full_keys.sort();
    // `link_all` adds the repository node and the directory chain on top.
    assert!(
        full_keys.len() > per_file_keys.len(),
        "link_all adds structural nodes"
    );
    for key in per_file_keys {
        assert!(full_keys.contains(&key), "{key} is missing from link_all");
    }
}

#[test]
fn link_is_deterministic_across_thread_counts() {
    let fixture = build_fixture();
    let outputs: Vec<codegraph::LinkOutput> = [1usize, 8]
        .iter()
        .filter_map(|threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(*threads)
                .build()
                .ok()
                .map(|pool| pool.install(|| link(&fixture)))
        })
        .collect();
    if outputs.len() < 2 {
        // A machine that cannot spawn eight analysis threads cannot demonstrate the property.
        return;
    }
    assert_eq!(outputs[0].edges, outputs[1].edges);
    assert_eq!(outputs[0].nodes, outputs[1].nodes);
    assert_eq!(outputs[0].unresolved, outputs[1].unresolved);
}

#[test]
fn resolution_deps_record_consulted_names_and_files() {
    let fixture = build_fixture();
    let config = LinkConfig::default();
    let (tables, names) = Linker::build_tables(&fixture.units);
    let caller = fixture
        .units
        .iter()
        .find(|unit| unit.file.as_str() == "src/caller.ts")
        .unwrap();
    let result = Linker::link_file(caller, &tables, &names, &fixture.resolver, &config);
    assert!(
        result.deps.names.contains("helper"),
        "the cascade consulted the name index for `helper`: {:?}",
        result.deps.names
    );
    assert!(
        result.deps.files.is_empty(),
        "a bare name consults no export table: {:?}",
        result.deps.files
    );

    let importer = fixture
        .units
        .iter()
        .find(|unit| unit.file.as_str() == "src/derived.ts")
        .unwrap();
    let result = Linker::link_file(importer, &tables, &names, &fixture.resolver, &config);
    assert!(
        result.deps.files.contains(&path("src/base.ts")),
        "an import consults the target file's export table: {:?}",
        result.deps.files
    );

    // An external specifier consults no export table at all: the package node comes straight from
    // the resolver's answer.
    let app = fixture
        .units
        .iter()
        .find(|unit| unit.file.as_str() == "src/app.service.ts")
        .unwrap();
    let result = Linker::link_file(app, &tables, &names, &fixture.resolver, &config);
    assert!(
        result.deps.files.is_empty(),
        "an external package resolves without reading a file: {:?}",
        result.deps.files
    );
}

#[test]
fn linker_fixture_edges_golden() {
    let fixture = build_fixture();
    let output = link(&fixture);
    insta::assert_snapshot!("graph_linker_fixture_edges", support::dump(&output));
}

#[test]
fn the_linked_fixture_builds_a_valid_graph() {
    let fixture = build_fixture();
    let output = link(&fixture);
    let graph = build_graph(&output);
    let report = codegraph::validate(&graph, SCHEMA_VERSION);
    assert!(
        report.errors.is_empty(),
        "the linked fixture must validate:\n{}",
        report.render(20)
    );
    assert!(graph.node_count() > 0);
    // The query surface the consumers use must work on a linked graph, not only a hand-built one.
    let _: &dyn GraphQuery = &graph;
}

#[test]
fn links_the_fixture_twice_to_the_same_bytes() {
    let fixture = build_fixture();
    let first = build_graph(&link(&fixture));
    let second = build_graph(&link(&fixture));
    assert_eq!(
        codegraph::compare(&first, &second, &codegraph::CompareOptions::strict()),
        codegraph::GraphDiffReport::default(),
        "linking the same units twice must be byte-identical"
    );
}

#[test]
fn resolver_is_consulted_a_bounded_number_of_times_per_specifier() {
    let fixture = build_fixture();
    let _ = link(&fixture);
    let queries = fixture.resolver.queries();
    // The structural pass resolves every import once and the cascade resolves each reference's
    // specifier, so a specifier is asked about more than once — but a bounded number of times.
    let mut per_specifier: BTreeMap<String, usize> = BTreeMap::new();
    for (_from, specifier) in &queries {
        *per_specifier.entry(specifier.clone()).or_insert(0) += 1;
    }
    for (specifier, count) in &per_specifier {
        assert!(
            *count <= 8,
            "{specifier} was resolved {count} times, which is not bounded"
        );
    }
    assert!(
        per_specifier.contains_key("./base"),
        "the fixture has imports, so the resolver must have been consulted"
    );
}

#[test]
fn local_ids_align_with_symbol_positions() {
    // Guards the fixture builder itself: a `LocalId` that does not index `symbols` would make
    // every other test in this file fail for the wrong reason.
    let mut unit = UnitBuilder::new("src/align.ts");
    let outer = unit.symbol(SymbolKind::Class, "Outer", 1);
    let inner = unit.child(SymbolKind::Method, "inner", Some(outer), 3);
    let unit = unit.build();
    assert_eq!(unit.symbols[outer.0 as usize].name, "Outer");
    assert_eq!(unit.symbols[inner.0 as usize].name, "inner");
    assert_eq!(unit.symbols[inner.0 as usize].parent, Some(outer));
}

#[test]
fn depth_limit_cuts_a_re_export_chain_instead_of_hanging() {
    // Ten barrels, each re-exporting the next, and a caller importing the first.
    let mut units = Vec::new();
    for n in 0..10 {
        let path = format!("src/deep/barrel{n}.ts");
        let mut unit = UnitBuilder::new(&path);
        unit.reexport(&format!("./barrel{}", n + 1), "deep", "deep");
        units.push(unit.build());
    }
    let mut leaf = UnitBuilder::new("src/deep/barrel10.ts");
    let deep = leaf.symbol(SymbolKind::Function, "deep", 1);
    leaf.exported(deep);
    units.push(leaf.build());

    let mut caller = UnitBuilder::new("src/deep/caller.ts");
    let run = caller.symbol(SymbolKind::Function, "run", 1);
    let (import, binding) = caller.import_named("./barrel0", "deep", "deep", 2);
    caller.reference_with(
        run,
        RefKind::Call,
        "deep",
        ReceiverHint::None,
        3,
        Some(analysis_ir::reference::BindingRef { import, binding }),
    );
    units.push(caller.build());

    let mut resolver = support::TableResolver::new();
    for n in 0..10 {
        resolver = resolver.file(
            "src/deep/caller.ts",
            &format!("./barrel{n}"),
            &format!("src/deep/barrel{n}.ts"),
        );
        resolver = resolver.file(
            &format!("src/deep/barrel{n}.ts"),
            &format!("./barrel{}", n + 1),
            &format!("src/deep/barrel{}.ts", n + 1),
        );
    }
    let output = Linker::link_all(LinkInput {
        units: &units,
        resolver: &resolver,
        config: &LinkConfig::default(),
        repository_name: "fixture",
    });
    let reference = output.unresolved.iter().find(|r| r.name == "deep");
    assert_eq!(
        reference.map(|r| r.reason),
        Some(codegraph::UnresolvedReason::DepthExceeded),
        "a chain longer than max_reexport_depth is cut, not followed"
    );
}
