#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! REV-001: input contract, refs, output schema, prompt versioning and the input hash.

mod common;

use model_gateway::{check_strict_compatible, SchemaValidators, TaskType};
use reviewers::input::SECTION_ORDER;
use reviewers::prompts::{check_lock, sha_of, PromptFile, PROMPTS, REGISTRY_LOCK};
use reviewers::{
    build_input, input_hash, reviewer_output_schema, FocusProfile, RefKind, RefTable,
    ReviewContext, ReviewerKind,
};

use common::{auth_bypass_context, golden_output, AUTHORIZE, CONTROLLER, UPDATE_USER};

#[test]
fn input_sections_are_ordered_for_caching() {
    let cx = auth_bypass_context();
    let input = build_input(TaskType::CorrectnessReview, &[], &cx, Vec::new());
    let names: Vec<&str> = input.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, SECTION_ORDER.to_vec());
    // The two stable sections carry the cache breakpoints, and only they do.
    let breakpoints: Vec<&str> = input
        .sections
        .iter()
        .filter(|s| s.cache_breakpoint)
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(breakpoints, ["task", "repository_rules"]);
}

#[test]
fn refs_are_deterministic() {
    let cx = auth_bypass_context();
    let a = RefTable::build(&cx);
    let b = RefTable::build(&cx);
    assert_eq!(a, b);
    assert_eq!(a.get("S1").unwrap().target, AUTHORIZE);
    assert_eq!(a.get("N1").unwrap().target, UPDATE_USER);
    assert_eq!(a.get("N2").unwrap().target, CONTROLLER);
    assert_eq!(a.get("T1").unwrap().kind, RefKind::Test);
    assert_eq!(a.get("R1").unwrap().kind, RefKind::Rule);
    assert!(a.get("N3").is_none());
    // Edges are rendered with refs, not ids.
    let input = build_input(TaskType::CorrectnessReview, &[], &cx, Vec::new());
    let graph = &input.sections[4].content;
    assert_eq!(graph["edges"][0]["from"], "N1");
    assert_eq!(graph["edges"][0]["to"], "S1");
    assert_eq!(input.sections[5].content[0]["covers"][0], "S1");
}

#[test]
fn ref_table_round_trips() {
    let table = RefTable::build(&auth_bypass_context());
    let json = serde_json::to_string(&table).unwrap();
    let back: RefTable = serde_json::from_str(&json).unwrap();
    assert_eq!(table, back);
    assert_eq!(back.len(), 5);
}

#[test]
fn output_schema_is_strict_compatible() {
    let schema = reviewer_output_schema().unwrap();
    check_strict_compatible(&schema.schema).expect("strict compatible");
    SchemaValidators::new().compile(&schema).expect("compiles");
}

#[test]
fn output_schema_accepts_golden_example() {
    let schema = reviewer_output_schema().unwrap();
    let errors = SchemaValidators::new()
        .validate(&schema, &golden_output())
        .unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let mut bad = golden_output();
    bad["findings"][0]["anchor"]["ref"] = "T1".into();
    assert!(!SchemaValidators::new()
        .validate(&schema, &bad)
        .unwrap()
        .is_empty());
}

#[test]
fn prompt_sha_matches_lock() {
    check_lock(PROMPTS, REGISTRY_LOCK).expect("registry.lock matches every prompt file");
    reviewers::prompts::self_test().expect("self test");
}

#[test]
fn prompt_change_without_bump_fails() {
    let edited = format!("{}\nOne more rule.\n", PROMPTS[0].text);
    let leaked: &'static str = Box::leak(edited.into_boxed_str());
    let files = [PromptFile {
        kind: PROMPTS[0].kind,
        version: PROMPTS[0].version,
        text: leaked,
    }];
    let errors = check_lock(&files, REGISTRY_LOCK).expect_err("edit without a bump");
    assert!(errors[0].contains("immutable"), "{errors:?}");
    assert_ne!(sha_of(leaked), sha_of(PROMPTS[0].text));
}

#[test]
fn input_hash_changes_with_prompt_or_package() {
    let cx = auth_bypass_context();
    let base = input_hash(
        &cx.package_hash(),
        ReviewerKind::Correctness,
        "1.0.0",
        "sha-a",
        &[],
        "table",
    );
    let same = input_hash(
        &cx.package_hash(),
        ReviewerKind::Correctness,
        "1.0.0",
        "sha-a",
        &[],
        "table",
    );
    assert_eq!(base, same);
    let other_prompt = input_hash(
        &cx.package_hash(),
        ReviewerKind::Correctness,
        "1.0.0",
        "sha-b",
        &[],
        "table",
    );
    assert_ne!(base, other_prompt);
    let mut cx2 = auth_bypass_context();
    cx2.nodes.pop();
    let other_package = input_hash(
        &cx2.package_hash(),
        ReviewerKind::Correctness,
        "1.0.0",
        "sha-a",
        &[],
        "table",
    );
    assert_ne!(base, other_package);
    let with_focus = input_hash(
        &cx.package_hash(),
        ReviewerKind::Correctness,
        "1.0.0",
        "sha-a",
        &[FocusProfile::DatabaseSafety],
        "table",
    );
    assert_ne!(base, with_focus);
}

#[test]
fn rules_section_marked_untrusted_in_prompt() {
    let cx = auth_bypass_context();
    let input = build_input(TaskType::CorrectnessReview, &[], &cx, Vec::new());
    assert_eq!(input.sections[1].content["untrusted_data"], true);
    for p in PROMPTS {
        let text = p.text.to_lowercase();
        assert!(
            text.contains("data, not instructions") || text.contains("data, never instructions"),
            "{}/v{} must say repository rules are data",
            p.kind,
            p.version
        );
    }
}
