#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use repository::frameworks::{
    auth_facts, detect_frameworks, to_framework_signals, EvidenceSource, FrameworkCategory,
    FrameworkFact,
};
use repository::manifests::detect_manifests;
use repository::read::BoundedReader;
use repository::walk::{walk, WalkOptions};
use review_test_support::{fixture_repo, write_file};

fn frameworks(root: &Path) -> Vec<FrameworkFact> {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (manifests, _) = detect_manifests(&inv, &reader);
    detect_frameworks(&inv, &manifests).0
}

fn find<'a>(facts: &'a [FrameworkFact], id: &str, scope: &str) -> Option<&'a FrameworkFact> {
    facts
        .iter()
        .find(|f| f.id == id && f.scope.as_str() == scope)
}

fn nest_tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "package.json",
        r#"{
  "name": "api",
  "dependencies": {
    "@nestjs/core": "^10.3.0",
    "@nestjs/common": "^10.3.0",
    "@nestjs/platform-express": "^10.3.0",
    "@nestjs/typeorm": "^10.0.0",
    "typeorm": "^0.3.20",
    "bullmq": "^5.1.0",
    "@nestjs/config": "^3.0.0",
    "@nestjs/jwt": "^10.0.0",
    "@nestjs/passport": "^10.0.0",
    "bull": "^4.0.0"
  },
  "devDependencies": { "jest": "^29.7.0", "ts-jest": "^29.1.0" },
  "jest": { "rootDir": "src" }
}"#,
    );
    write_file(tmp.path(), "nest-cli.json", "{}");
    write_file(tmp.path(), "src/data-source.ts", "export const ds = {};\n");
    tmp
}

#[test]
fn nestjs_detected_with_major() {
    let tmp = nest_tree();
    let facts = frameworks(tmp.path());
    let nest = find(&facts, "nestjs", "").unwrap();
    assert_eq!(nest.major, Some(10));
    assert_eq!(nest.category, FrameworkCategory::Web);
    // dependency + nest-cli.json
    assert_eq!(nest.confidence, 1.0);
    assert!(nest.via.is_none());
}

#[test]
fn express_via_nest_platform_not_standalone() {
    let tmp = nest_tree();
    let facts = frameworks(tmp.path());
    let express = find(&facts, "express", "").unwrap();
    assert_eq!(express.via.as_deref(), Some("nestjs"));
    assert_eq!(express.confidence, 0.9);

    let standalone = tempfile::tempdir().unwrap();
    write_file(
        standalone.path(),
        "package.json",
        r#"{"name":"s","dependencies":{"express":"^4.19.0"}}"#,
    );
    let facts = frameworks(standalone.path());
    assert!(find(&facts, "express", "").unwrap().via.is_none());
}

#[test]
fn nextjs_and_react_in_web_package_scope() {
    let repo = fixture_repo("monorepo-pnpm");
    let facts = frameworks(&repo);
    let next = find(&facts, "nextjs", "apps/web").unwrap();
    assert_eq!(next.major, Some(14));
    assert_eq!(next.confidence, 1.0, "next.config.js is present");
    assert!(find(&facts, "react", "apps/web").is_some());
    assert!(find(&facts, "nextjs", "").is_none());
    assert!(find(&facts, "nestjs", "apps/api").is_some());
    assert!(find(&facts, "typeorm", "packages/db").is_some());
}

#[test]
fn typeorm_by_dependency_and_data_source() {
    let tmp = nest_tree();
    let facts = frameworks(tmp.path());
    let typeorm = find(&facts, "typeorm", "").unwrap();
    assert_eq!(typeorm.confidence, 1.0);
    assert!(
        typeorm
            .evidence
            .iter()
            .any(|e| e.source == EvidenceSource::ConfigFile
                && e.path.as_str() == "src/data-source.ts")
    );
}

#[test]
fn prisma_by_schema_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "package.json", r#"{"name":"p"}"#);
    write_file(
        tmp.path(),
        "prisma/schema.prisma",
        "model A { id Int @id }\n",
    );
    let facts = frameworks(tmp.path());
    let prisma = find(&facts, "prisma", "").unwrap();
    assert_eq!(prisma.confidence, 0.7);
    assert!(prisma
        .evidence
        .iter()
        .any(|e| e.source == EvidenceSource::SchemaFile));
}

#[test]
fn bullmq_vs_legacy_bull() {
    let tmp = nest_tree();
    let facts = frameworks(tmp.path());
    assert!(find(&facts, "bullmq", "").is_some());
    assert!(find(&facts, "bull", "").is_some());
    let only_mq = tempfile::tempdir().unwrap();
    write_file(
        only_mq.path(),
        "package.json",
        r#"{"name":"q","dependencies":{"@nestjs/bullmq":"^10.0.0"}}"#,
    );
    let facts = frameworks(only_mq.path());
    assert!(find(&facts, "bullmq", "").is_some());
    assert!(find(&facts, "bull", "").is_none());
}

#[test]
fn jest_from_package_json_key() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "package.json",
        r#"{"name":"j","jest":{"rootDir":"src"}}"#,
    );
    let facts = frameworks(tmp.path());
    let jest = find(&facts, "jest", "").unwrap();
    assert_eq!(jest.confidence, 0.7);
    assert_eq!(jest.category, FrameworkCategory::Test);
}

#[test]
fn vitest_config_only_confidence_0_7() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "package.json", r#"{"name":"v"}"#);
    write_file(tmp.path(), "vitest.config.ts", "export default {};\n");
    let facts = frameworks(tmp.path());
    assert_eq!(find(&facts, "vitest", "").unwrap().confidence, 0.7);
}

#[test]
fn runtime_framework_in_dev_deps_lower_confidence() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "package.json",
        r#"{"name":"d","devDependencies":{"react":"^18.0.0","vitest":"^1.0.0"}}"#,
    );
    let facts = frameworks(tmp.path());
    assert_eq!(find(&facts, "react", "").unwrap().confidence, 0.6);
    // test frameworks in devDependencies are the normal case
    assert_eq!(find(&facts, "vitest", "").unwrap().confidence, 0.9);
}

#[test]
fn auth_libraries_categorized() {
    let tmp = nest_tree();
    let facts = frameworks(tmp.path());
    let auth = auth_facts(&facts);
    assert_eq!(auth.libraries, vec!["jwt", "passport"]);
    assert_eq!(
        find(&facts, "jwt", "").unwrap().category,
        FrameworkCategory::Auth
    );
    assert!(find(&facts, "config", "").is_some());
}

#[test]
fn framework_signals_conversion_roundtrip() {
    let repo = fixture_repo("monorepo-pnpm");
    let facts = frameworks(&repo);
    let signals = to_framework_signals(&facts);
    let nest = &signals.frameworks["nestjs"];
    assert_eq!(nest.major, Some(10));
    assert_eq!(nest.scope_dirs.len(), 1);
    assert_eq!(nest.scope_dirs[0].as_str(), "apps/api");
    let json = serde_json::to_string(&signals).unwrap();
    let back: repository::frameworks::FrameworkSignalsData = serde_json::from_str(&json).unwrap();
    assert_eq!(back, signals);
}

#[test]
#[ignore = "needs RG_REFERENCE_REPO_PATH"]
fn reference_frameworks() {
    let root = std::env::var("RG_REFERENCE_REPO_PATH").unwrap();
    let facts = frameworks(Path::new(&root));
    for id in ["nestjs", "typeorm", "bullmq", "jest", "config"] {
        assert!(facts.iter().any(|f| f.id == id), "{id}");
    }
    assert!(!auth_facts(&facts).libraries.is_empty());
    let express = facts.iter().find(|f| f.id == "express").unwrap();
    assert_eq!(express.via.as_deref(), Some("nestjs"));
}
