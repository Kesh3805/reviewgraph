#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::Utc;
use common::*;
use context_engine::candidates::api::{ApiCandidates, ApiChange};
use context_engine::candidates::config::{ConfigCandidates, ConfigFile, ConfigKey};
use context_engine::candidates::rules::{RuleDocCandidates, RuleEntry};
use context_engine::candidates::tests::{TestCandidates, MOCKED_FACTOR};
use context_engine::candidates::{CandidateGenerator, CandidatePool, ChangedSeed};
use context_engine::model::ItemKind;
use impact::Relation;
use profile::model::{Convention, ConventionSource};
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

fn generate(
    g: &dyn CandidateGenerator,
    input: &context_engine::candidates::SelectionInput<'_>,
) -> CandidatePool {
    let mut pool = CandidatePool::new();
    g.generate(input, &mut pool);
    pool
}

#[test]
fn auth_bypass_authorize_spec_candidate() {
    let graph = graph_of(auth_bypass_elements());
    let input = input(ReviewerType::Correctness, Some(&graph));
    let pool = generate(&TestCandidates, &input);
    let spec = pool.get_node(key(SPEC), DiffSide::Head).unwrap();
    assert_eq!(spec.kind, ItemKind::Test);
    assert!((spec.raw.test_relationship - 0.9).abs() < 1e-6);
}

#[test]
fn mocked_test_penalized() {
    let graph = graph_of(vec![test_element(0.8, true)]);
    let input = input(ReviewerType::Test, Some(&graph));
    let pool = generate(&TestCandidates, &input);
    let spec = pool.get_node(key(SPEC), DiffSide::Head).unwrap();
    assert!((spec.raw.test_relationship - 0.8 * MOCKED_FACTOR).abs() < 1e-6);
}

#[test]
fn low_score_tests_are_not_candidates() {
    let graph = graph_of(vec![test_element(0.5, false)]);
    let input = input(ReviewerType::Test, Some(&graph));
    assert!(generate(&TestCandidates, &input).is_empty());
}

#[test]
fn changed_test_pinned() {
    let mut test_seed: ChangedSeed = seed();
    test_seed.key = key(SPEC);
    test_seed.title = SPEC.to_owned();
    test_seed.is_test = true;
    let mut input = input(ReviewerType::Test, None);
    input.seeds = vec![test_seed];
    let pool = generate(&TestCandidates, &input);
    let c = pool.get_node(key(SPEC), DiffSide::Head).unwrap();
    assert!(c.pinned);
    assert_eq!(c.kind, ItemKind::Test);
}

#[test]
fn env_values_never_included() {
    let mut input = input(ReviewerType::Security, None);
    input.config_files = vec![
        ConfigFile {
            path: path(".env.production"),
            keys: vec![ConfigKey {
                key: "LOG_LEVEL".into(),
                line: 1,
                value: Some("debug".into()),
            }],
        },
        ConfigFile {
            path: path("config/app.yaml"),
            keys: vec![
                ConfigKey {
                    key: "port".into(),
                    line: 2,
                    value: Some("8080".into()),
                },
                ConfigKey {
                    key: "db_password".into(),
                    line: 3,
                    value: Some("hunter2".into()),
                },
            ],
        },
    ];
    let pool = generate(&ConfigCandidates, &input);
    let rendered: Vec<String> = pool
        .iter()
        .map(|c| c.structured.as_ref().unwrap().to_string())
        .collect();
    let all = rendered.join("\n");
    assert!(!all.contains("debug"), "env file value leaked: {all}");
    assert!(!all.contains("hunter2"), "secret value leaked: {all}");
    assert!(all.contains("8080"), "plain config value kept: {all}");
    assert!(all.contains("LOG_LEVEL"));
}

#[test]
fn endpoint_candidate_has_guards_and_dto() {
    let graph = graph_of(auth_bypass_elements());
    let mut input = input(ReviewerType::Security, Some(&graph));
    input.api_changes = vec![ApiChange {
        endpoint: "PUT /users/:id".into(),
        handler: Some(key(CONTROLLER)),
        related: vec![key(FORMAT_HEADER)],
        breaking: false,
    }];
    let pool = generate(&ApiCandidates, &input);
    let ep = pool.get_node(key(ENDPOINT), DiffSide::Head).unwrap();
    assert_eq!(ep.kind, ItemKind::ApiEndpoint);
    assert!(ep.pinned, "security pins the reached endpoint");
    let value = ep.structured.as_ref().unwrap();
    assert_eq!(value["guards"][0], "JwtGuard");
    assert_eq!(value["method"], "PUT");
    assert_eq!(value["dto_types"].as_array().unwrap().len(), 1);
    let dto = pool.get_node(key(FORMAT_HEADER), DiffSide::Head).unwrap();
    assert!((dto.raw.api_relationship - 1.0).abs() < 1e-6);
}

fn convention(id: &str, source: ConventionSource, confidence: f32, samples: u32) -> RuleEntry {
    RuleEntry::from_convention(&Convention {
        id: id.into(),
        rule: format!("rule text of {id}"),
        scope: "src/**".into(),
        samples,
        violations: 0,
        consistency: 1.0,
        confidence,
        exceptions: vec![],
        source,
        computed_at: Utc::now(),
        profile_version: 1,
    })
}

#[test]
fn policy_rule_beats_inferred_convention() {
    let mut input = input(ReviewerType::Security, None);
    input.rules = vec![
        convention("inferred", ConventionSource::Inferred, 0.95, 20),
        convention("policy", ConventionSource::Explicit, 0.5, 1),
        convention("documented", ConventionSource::Documented, 0.5, 1),
    ];
    let pool = generate(&RuleDocCandidates, &input);
    let prio = |id: &str| {
        pool.iter()
            .find(|c| c.title == id)
            .unwrap()
            .raw
            .rule_priority
    };
    assert!(prio("policy") > prio("documented"));
    assert!(prio("documented") > prio("inferred"));
    assert_eq!(
        pool.iter().find(|c| c.title == "policy").unwrap().kind,
        ItemKind::RuleDoc
    );
    assert_eq!(
        pool.iter().find(|c| c.title == "inferred").unwrap().kind,
        ItemKind::Convention
    );
}

#[test]
fn low_sample_convention_excluded() {
    let mut input = input(ReviewerType::Security, None);
    input.rules = vec![
        convention("few-samples", ConventionSource::Inferred, 0.95, 9),
        convention("low-confidence", ConventionSource::Inferred, 0.85, 50),
    ];
    assert!(generate(&RuleDocCandidates, &input).is_empty());
}

#[test]
fn rules_outside_scope_or_without_profile_are_skipped() {
    let mut input = input(ReviewerType::Security, None);
    let mut other = convention("other", ConventionSource::Explicit, 1.0, 100);
    other.scope = "docs/**".into();
    input.rules = vec![other];
    assert!(generate(&RuleDocCandidates, &input).is_empty());
    input.profile_available = false;
    let mut pool = CandidatePool::new();
    assert!(RuleDocCandidates.generate(&input, &mut pool).degraded);
}

#[test]
fn config_elements_from_impact_are_key_level() {
    let el = element(
        "env:DATABASE_URL",
        codegraph::NodeKind::EnvironmentVariable,
        Relation::EnvVar,
        1,
        1.0,
    );
    let graph = graph_of(vec![el]);
    let input = input(ReviewerType::Correctness, Some(&graph));
    let pool = generate(&ConfigCandidates, &input);
    let c = pool
        .get_node(key("env:DATABASE_URL"), DiffSide::Head)
        .unwrap();
    assert_eq!(c.kind, ItemKind::Config);
    assert!((c.raw.config_relationship - 1.0).abs() < 1e-6);
    assert!(!c.structured.as_ref().unwrap().to_string().contains("value"));
}
