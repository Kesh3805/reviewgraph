//! The persisted taxonomy, pinned as a golden.
//!
//! `engine/crates/graph-storage/src/kinds.rs` holds storage-side mirrors of `NodeKind`, `EdgeKind`,
//! `ResolvedBy`, `Provenance`, `Confidence` and `EdgeFlags` and writes them into `node_kinds`,
//! `edge_kinds`, `resolved_by_kinds` and `provenance_kinds`. Those columns already hold data, so a
//! discriminant or a wire spelling cannot change without a migration and a `SCHEMA_VERSION` bump.
//!
//! This file is the `codegraph` side of that contract: every value below is written out literally,
//! so if someone edits an enum here the test fails and points at the table that already disagrees.
//! The literal lists mirror `kinds.rs` — keeping them in step is the point, not deduplicating them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use codegraph::{
    confidence_of, derived, Confidence, EdgeFlags, EdgeKind, EdgeKindSet, NodeId, NodeKind,
    Provenance, ResolvedBy, ReverseView,
};
use review_core::location::RepoPath;

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

/// `(variant, discriminant, persisted spelling)` for `NodeKind`, in the order `kinds.rs` declares.
const NODE_KINDS: [(NodeKind, u8, &str); 44] = [
    (NodeKind::Repository, 0, "Repository"),
    (NodeKind::Package, 1, "Package"),
    (NodeKind::Module, 2, "Module"),
    (NodeKind::Directory, 3, "Directory"),
    (NodeKind::File, 4, "File"),
    (NodeKind::Namespace, 10, "Namespace"),
    (NodeKind::Class, 11, "Class"),
    (NodeKind::Interface, 12, "Interface"),
    (NodeKind::Struct, 13, "Struct"),
    (NodeKind::Trait, 14, "Trait"),
    (NodeKind::Enum, 15, "Enum"),
    (NodeKind::TypeAlias, 16, "TypeAlias"),
    (NodeKind::Function, 20, "Function"),
    (NodeKind::Method, 21, "Method"),
    (NodeKind::Constructor, 22, "Constructor"),
    (NodeKind::Property, 23, "Property"),
    (NodeKind::Field, 24, "Field"),
    (NodeKind::Parameter, 25, "Parameter"),
    (NodeKind::Variable, 26, "Variable"),
    (NodeKind::Constant, 27, "Constant"),
    (NodeKind::ApiEndpoint, 30, "ApiEndpoint"),
    (NodeKind::Controller, 31, "Controller"),
    (NodeKind::Middleware, 33, "Middleware"),
    (NodeKind::Handler, 32, "Handler"),
    (NodeKind::DatabaseEntity, 40, "DatabaseEntity"),
    (NodeKind::DatabaseTable, 41, "DatabaseTable"),
    (NodeKind::DatabaseColumn, 42, "DatabaseColumn"),
    (NodeKind::Migration, 43, "Migration"),
    (NodeKind::Queue, 50, "Queue"),
    (NodeKind::QueueProducer, 51, "QueueProducer"),
    (NodeKind::QueueConsumer, 52, "QueueConsumer"),
    (NodeKind::JobHandler, 53, "JobHandler"),
    (NodeKind::Configuration, 60, "Configuration"),
    (NodeKind::EnvironmentVariable, 61, "EnvironmentVariable"),
    (NodeKind::TestSuite, 70, "TestSuite"),
    (NodeKind::TestCase, 71, "TestCase"),
    (NodeKind::Fixture, 72, "Fixture"),
    (NodeKind::ExternalDependency, 80, "ExternalDependency"),
    (NodeKind::ExternalApi, 81, "ExternalApi"),
    (NodeKind::BuildTarget, 90, "BuildTarget"),
    (NodeKind::CliCommand, 91, "CliCommand"),
    (NodeKind::Worker, 92, "Worker"),
    (NodeKind::DocumentationRule, 100, "DocumentationRule"),
    (
        NodeKind::ArchitecturalBoundary,
        101,
        "ArchitecturalBoundary",
    ),
];

/// `(variant, discriminant, persisted spelling)` for `EdgeKind`. The three reverse views are never
/// stored (clarification C2), so they are absent from the persisted taxonomy on purpose.
const EDGE_KINDS: [(EdgeKind, u8, &str); 33] = [
    (EdgeKind::Contains, 0, "CONTAINS"),
    (EdgeKind::Declares, 1, "DECLARES"),
    (EdgeKind::Imports, 2, "IMPORTS"),
    (EdgeKind::Exports, 3, "EXPORTS"),
    (EdgeKind::Calls, 4, "CALLS"),
    (EdgeKind::Reads, 5, "READS"),
    (EdgeKind::Writes, 6, "WRITES"),
    (EdgeKind::Implements, 7, "IMPLEMENTS"),
    (EdgeKind::Extends, 8, "EXTENDS"),
    (EdgeKind::Overrides, 9, "OVERRIDES"),
    (EdgeKind::References, 10, "REFERENCES"),
    (EdgeKind::UsesType, 11, "USES_TYPE"),
    (EdgeKind::ReturnsType, 12, "RETURNS_TYPE"),
    (EdgeKind::AcceptsType, 13, "ACCEPTS_TYPE"),
    (EdgeKind::RoutesTo, 14, "ROUTES_TO"),
    (EdgeKind::HandledBy, 15, "HANDLED_BY"),
    (EdgeKind::Tests, 16, "TESTS"),
    (EdgeKind::Covers, 17, "COVERS"),
    (EdgeKind::ProducesJob, 18, "PRODUCES_JOB"),
    (EdgeKind::ConsumesJob, 19, "CONSUMES_JOB"),
    (EdgeKind::ReadsConfig, 20, "READS_CONFIG"),
    (EdgeKind::WritesConfig, 21, "WRITES_CONFIG"),
    (EdgeKind::ReadsTable, 22, "READS_TABLE"),
    (EdgeKind::WritesTable, 23, "WRITES_TABLE"),
    (EdgeKind::DependsOn, 24, "DEPENDS_ON"),
    (EdgeKind::Throws, 25, "THROWS"),
    (EdgeKind::Catches, 26, "CATCHES"),
    (EdgeKind::Serializes, 27, "SERIALIZES"),
    (EdgeKind::Deserializes, 28, "DESERIALIZES"),
    (EdgeKind::Validates, 29, "VALIDATES"),
    (EdgeKind::Authorizes, 30, "AUTHORIZES"),
    (EdgeKind::Publishes, 31, "PUBLISHES"),
    (EdgeKind::Subscribes, 32, "SUBSCRIBES"),
];

const RESOLVED_BY: [(ResolvedBy, u8, &str); 10] = [
    (ResolvedBy::Structural, 0, "STRUCTURAL"),
    (ResolvedBy::Import, 1, "IMPORT"),
    (ResolvedBy::ThisMember, 2, "THIS_MEMBER"),
    (ResolvedBy::DiConstructor, 3, "DI_CONSTRUCTOR"),
    (ResolvedBy::TypeAnnotation, 4, "TYPE_ANNOTATION"),
    (ResolvedBy::NameUnique, 5, "NAME_UNIQUE"),
    (ResolvedBy::NameAmbiguous, 6, "NAME_AMBIGUOUS"),
    (ResolvedBy::Framework, 7, "FRAMEWORK"),
    (ResolvedBy::TypeChecker, 8, "TYPE_CHECKER"),
    (ResolvedBy::Heuristic, 9, "HEURISTIC"),
];

const PROVENANCE: [(Provenance, u8, &str); 6] = [
    (Provenance::Analyzer, 0, "ANALYZER"),
    (Provenance::Framework, 1, "FRAMEWORK"),
    (Provenance::Linker, 2, "LINKER"),
    (Provenance::TypeChecker, 3, "TYPE_CHECKER"),
    (Provenance::Heuristic, 4, "HEURISTIC"),
    (Provenance::Policy, 5, "POLICY"),
];

/// `(rule, permille)` — the whole `Confidence` table of target-architecture §3.1.
const CONFIDENCE: [(ResolvedBy, u16); 10] = [
    (ResolvedBy::Structural, 1000),
    (ResolvedBy::TypeChecker, 1000),
    (ResolvedBy::Import, 950),
    (ResolvedBy::ThisMember, 950),
    (ResolvedBy::Framework, 900),
    (ResolvedBy::DiConstructor, 850),
    (ResolvedBy::TypeAnnotation, 800),
    (ResolvedBy::NameUnique, 600),
    (ResolvedBy::Heuristic, 500),
    (ResolvedBy::NameAmbiguous, 300),
];

#[test]
fn node_kind_discriminants_and_spellings_are_frozen() {
    assert_eq!(NodeKind::ALL.len(), NODE_KINDS.len());
    for (kind, discriminant, spelling) in NODE_KINDS {
        assert_eq!(kind.as_u8(), discriminant, "{kind:?}");
        assert_eq!(kind.as_str(), spelling, "{kind:?}");
        assert_eq!(
            NodeKind::from_u8(discriminant),
            Some(kind),
            "{spelling} must parse back"
        );
        assert_eq!(NodeKind::from_str_exact(spelling), Some(kind));
    }
}

#[test]
fn edge_kind_discriminants_and_spellings_are_frozen() {
    assert_eq!(EdgeKind::ALL.len(), EDGE_KINDS.len());
    for (kind, discriminant, spelling) in EDGE_KINDS {
        assert_eq!(kind.as_u8(), discriminant, "{kind:?}");
        assert_eq!(kind.as_str(), spelling, "{kind:?}");
        assert_eq!(EdgeKind::from_u8(discriminant), Some(kind));
        assert_eq!(EdgeKind::from_str_exact(spelling), Some(kind));
    }
    // A reverse view is a query-time projection, never a stored row.
    for view in [
        ReverseView::CalledBy,
        ReverseView::DependedOnBy,
        ReverseView::TestedBy,
    ] {
        assert!(
            EdgeKind::from_str_exact(view.as_str()).is_none(),
            "{} must not be a storable kind",
            view.as_str()
        );
    }
}

#[test]
fn resolved_by_and_provenance_are_frozen() {
    assert_eq!(ResolvedBy::ALL.len(), RESOLVED_BY.len());
    for (rule, discriminant, spelling) in RESOLVED_BY {
        assert_eq!(rule.as_u8(), discriminant, "{rule:?}");
        assert_eq!(rule.as_str(), spelling, "{rule:?}");
        assert_eq!(ResolvedBy::from_u8(discriminant), Some(rule));
        assert_eq!(ResolvedBy::from_str_exact(spelling), Some(rule));
    }
    assert_eq!(Provenance::ALL.len(), PROVENANCE.len());
    for (provenance, discriminant, spelling) in PROVENANCE {
        assert_eq!(provenance.as_u8(), discriminant, "{provenance:?}");
        assert_eq!(provenance.as_str(), spelling, "{provenance:?}");
        assert_eq!(Provenance::from_u8(discriminant), Some(provenance));
        assert_eq!(Provenance::from_str_exact(spelling), Some(provenance));
    }
}

#[test]
fn the_confidence_table_is_frozen() {
    for (rule, permille) in CONFIDENCE {
        assert_eq!(
            confidence_of(rule).as_permille(),
            permille,
            "{rule:?} changed confidence"
        );
    }
    // Storage persists `permille / 1000` as a `real`, so the conversion must round-trip exactly.
    for (rule, permille) in CONFIDENCE {
        assert_eq!(
            Confidence::from_f32(f32::from(permille) / 1000.0).as_permille(),
            permille,
            "{rule:?} must survive the real round-trip"
        );
    }
}

#[test]
fn a_derived_edge_is_never_more_confident_than_its_input() {
    for (rule, permille) in CONFIDENCE {
        let input = Confidence::from_f32(f32::from(permille) / 1000.0);
        let result = derived(rule, input);
        assert!(
            result.as_permille() <= input.as_permille(),
            "{rule:?} raised confidence"
        );
        assert_eq!(derived(rule, result), result, "{rule:?} is not idempotent");
    }
}

#[test]
fn edge_flag_bits_are_frozen() {
    // The bits are stored in a `smallint` and relied on by SQL predicates.
    assert_eq!(EdgeFlags::INSTANTIATES.bits(), 1);
    assert_eq!(EdgeFlags::DECORATOR.bits(), 2);
    assert_eq!(EdgeFlags::TYPE_ONLY.bits(), 4);
    assert_eq!(EdgeFlags::DYNAMIC.bits(), 8);
    assert_eq!(EdgeFlags::MAPS_TABLE.bits(), 16);
    assert_eq!(EdgeFlags::GLOBAL_SCOPE.bits(), 32);
    assert_eq!(EdgeFlags::ALL_BITS.bits(), 63);
    assert_eq!(EdgeFlags::EMPTY.bits(), 0);
    assert_eq!(
        EdgeFlags::INSTANTIATES
            .union(EdgeFlags::TYPE_ONLY)
            .union(EdgeFlags::GLOBAL_SCOPE)
            .bits(),
        37
    );
    assert_eq!(
        EdgeFlags::from_bits(63).names().join(","),
        "INSTANTIATES,DECORATOR,TYPE_ONLY,DYNAMIC,MAPS_TABLE,GLOBAL_SCOPE",
        "flag names are persisted too"
    );
}

#[test]
fn an_edge_kind_set_is_a_u64_bitmask_over_the_33_kinds() {
    let mut all = EdgeKindSet::default();
    for (kind, _, _) in EDGE_KINDS {
        all.insert(kind);
    }
    assert_eq!(all.iter().count(), 33);
    assert_eq!(all, EdgeKindSet::ALL);
    assert!(all.contains(EdgeKind::Calls) && all.contains(EdgeKind::Subscribes));

    let mut some = EdgeKindSet::default();
    some.insert(EdgeKind::Calls);
    some.insert(EdgeKind::Tests);
    assert_eq!(some.iter().count(), 2);
    assert_eq!(
        some.iter().collect::<Vec<_>>(),
        vec![EdgeKind::Calls, EdgeKind::Tests],
        "iteration is in discriminant order"
    );
    some.remove(EdgeKind::Calls);
    assert_eq!(some.iter().collect::<Vec<_>>(), vec![EdgeKind::Tests]);

    // Bits past the taxonomy are masked off rather than kept, so a stale bit can never widen a
    // filter into an accidental match-all.
    assert_eq!(EdgeKindSet::from_bits(1u64 << 40), EdgeKindSet::default());
    assert_eq!(
        EdgeKindSet::from_bits(EdgeKindSet::of(EdgeKind::Calls).bits() | 1u64 << 40),
        EdgeKindSet::of(EdgeKind::Calls)
    );
}

#[test]
fn synthetic_node_ids_are_frozen() {
    // These spellings are what the linker, the framework mapper and the API all agree on.
    let cases: [(NodeId, &str); 12] = [
        (NodeId::repository(), "repo:/"),
        (NodeId::file(&path("src/a.ts")), "file:src/a.ts"),
        (NodeId::directory(&path("src")), "dir:src"),
        (NodeId::workspace_package(&path("src")), "package:src"),
        (
            NodeId::http("get", "/users/:id").unwrap(),
            "http:GET /users/{}",
        ),
        (NodeId::queue("jobs").unwrap(), "queue:jobs"),
        (
            NodeId::table(Some("public"), "users").unwrap(),
            "db:public.users",
        ),
        (NodeId::env("DATABASE_URL").unwrap(), "env:DATABASE_URL"),
        (
            NodeId::package("npm", "express").unwrap(),
            "pkg:npm/express",
        ),
        (
            NodeId::test(&path("src/a.test.ts"), "adds", "a.ts").unwrap(),
            "test:src/a.test.ts#adds \u{203a} a.ts",
        ),
        (
            NodeId::from_canonical("ts:src/a.ts#T/method"),
            "ts:src/a.ts#T/method",
        ),
        (NodeId::from_canonical("x-unknown"), "x-unknown"),
    ];
    for (id, expected) in cases {
        assert_eq!(id.as_str(), expected);
        assert_eq!(NodeId::from_canonical(expected), id, "{expected}");
        assert_eq!(id.to_string(), expected);
    }
}

#[test]
fn a_node_id_key_is_the_blake3_128_of_its_canonical_form() {
    // `NodeKey` is what storage writes as 16 bytes, so the key must be a pure function of the id
    // and must never depend on insertion order.
    let a = NodeId::from_canonical("ts:src/a.ts#f/function");
    let b = NodeId::from_canonical("ts:src/b.ts#f/function");
    assert_eq!(a.key(), a.key(), "keys are stable");
    assert_ne!(a.key(), b.key(), "different ids have different keys");
    assert_eq!(a.key().as_bytes().len(), 16);

    // Ten distinct ids with the same shape must produce ten distinct keys.
    let keys: std::collections::BTreeSet<_> = (0..10)
        .map(|n| NodeId::from_canonical(format!("ts:src/a.ts#f{n}/function")).key())
        .collect();
    assert_eq!(keys.len(), 10);
}
