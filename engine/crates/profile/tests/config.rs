//! POL-001: `.review/config.yaml` schema, parser and validation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::NaiveDate;
use profile::config::{
    load_config_at, ConfigIssueKind, ConfigStatus, KnowledgeSourceKind, ReviewConfigV1, RiskLevel,
    RuleSeverity, SuppressionKind,
};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
}

fn load(text: &str) -> profile::config::LoadedConfig {
    load_config_at(Some(text.as_bytes()), today())
}

/// The PRD §122 example, verbatim in shape.
const PRD_122: &str = r#"
version: 1
review:
  reviewers: { correctness: true, security: true, tests: true, performance: true, architecture: true, maintainability: false }
  confidence: { minimum_publish: 0.72, per_reviewer: { maintainability: 0.90 } }
  budgets: { max_symbols: 100, max_context_tokens: 40000, max_model_calls: 30, max_review_seconds: 300 }
  generated: { ignore: ["**/*.generated.ts"] }
  risk: { paths: { "src/auth/**": critical, "migrations/**": high } }
  privacy: { external_models: true }
  publish: { inline_cap: 25, summary: true, check_run: true }
architecture:
  layers: { controllers: ["src/**/*.controller.ts"], repositories: ["src/**/*.repository.ts"] }
rules:
  forbidden_dependencies: [ { id: no-ctrl-repo, from: controllers, to: repositories, severity: high, reason: "Controllers go through services." } ]
  queue_jobs: { require_deterministic_id: true }
  database: { migrations_only: true, migration_paths: ["migrations/**", "src/migrations/**"] }
  tests: { public_api_changes_require_tests: true }
  security: { authorization_symbols: ["PermissionService.check"] }
conventions: { exceptions: ["src/legacy/**"] }
suppressions: [ { id: s1, type: path, value: "src/legacy/**", reason: "Legacy code is frozen.", owner: "@team", expires: 2027-01-01 } ]
knowledge_sources: [ { id: reference, kind: markdown_vault, path: .agent/knowledge }, { id: adr, kind: adr, path: docs/decisions } ]
"#;

#[test]
fn prd_122_example_parses() {
    let loaded = load(PRD_122);
    assert_eq!(loaded.status, ConfigStatus::Valid, "{:?}", loaded.issues);
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    let c = &loaded.config;
    assert!(!c.review.reviewers.maintainability);
    assert_eq!(c.review.confidence.minimum_publish, 0.72);
    assert_eq!(c.review.confidence.per_reviewer["maintainability"], 0.90);
    assert_eq!(c.review.budgets.max_context_tokens, 40_000);
    assert_eq!(c.review.risk.paths["src/auth/**"], RiskLevel::Critical);
    assert_eq!(c.review.generated.ignore, vec!["**/*.generated.ts"]);
    assert_eq!(
        c.architecture.layers["controllers"],
        vec!["src/**/*.controller.ts"]
    );
    assert_eq!(c.suppressions[0].kind, SuppressionKind::Path);
    assert_eq!(
        c.suppressions[0].expires,
        NaiveDate::from_ymd_opt(2027, 1, 1)
    );
    assert_eq!(
        c.knowledge_sources[0].kind,
        KnowledgeSourceKind::MarkdownVault
    );
    assert_eq!(c.knowledge_sources[1].path, "docs/decisions");
}

#[test]
fn prd_66_rules_parse() {
    let rules = load(PRD_122).config.rules;
    assert_eq!(rules.forbidden_dependencies.len(), 1);
    let rule = &rules.forbidden_dependencies[0];
    assert_eq!(
        (rule.id.as_str(), rule.from.as_str(), rule.to.as_str()),
        ("no-ctrl-repo", "controllers", "repositories")
    );
    assert_eq!(rule.severity, RuleSeverity::High);
    assert!(rules.queue_jobs.require_deterministic_id);
    assert!(rules.database.migrations_only);
    assert_eq!(rules.database.migration_paths.len(), 2);
    assert!(rules.tests.public_api_changes_require_tests);
    assert_eq!(
        rules.security.authorization_symbols,
        vec!["PermissionService.check"]
    );
}

#[test]
fn unknown_key_error_with_suggestion() {
    let loaded = load("version: 1\nrules:\n  queue_job: { require_deterministic_id: true }\n");
    assert_eq!(loaded.status, ConfigStatus::Invalid);
    let issue = loaded
        .errors()
        .find(|i| i.kind == ConfigIssueKind::UnknownKey)
        .unwrap();
    assert_eq!(issue.path, "rules.queue_job");
    assert!(
        issue.message.contains("did you mean queue_jobs"),
        "{}",
        issue.message
    );
    // review doctor reports it with its key path.
    assert!(loaded
        .doctor_report()
        .iter()
        .any(|line| line.starts_with("error rules.queue_job:")));
}

#[test]
fn unknown_nested_key_inside_list_is_located() {
    let loaded = load("version: 1\nsuppressions: [ { id: a, typ: path, value: x, reason: r } ]\n");
    let issue = loaded.errors().next().unwrap();
    assert_eq!(issue.path, "suppressions[0].typ");
    assert!(
        issue.message.contains("did you mean type"),
        "{}",
        issue.message
    );
}

#[test]
fn minimum_publish_below_floor_rejected() {
    let loaded = load("version: 1\nreview:\n  confidence: { minimum_publish: 0.4 }\n");
    assert_eq!(loaded.status, ConfigStatus::Invalid);
    let issue = loaded.errors().next().unwrap();
    assert_eq!(issue.kind, ConfigIssueKind::OutOfRange);
    assert_eq!(issue.path, "review.confidence.minimum_publish");

    let maint =
        load("version: 1\nreview:\n  confidence: { per_reviewer: { maintainability: 0.7 } }\n");
    assert_eq!(maint.status, ConfigStatus::Invalid);
    assert_eq!(
        maint.errors().next().unwrap().path,
        "review.confidence.per_reviewer.maintainability"
    );
}

#[test]
fn undefined_layer_rejected() {
    let loaded = load(
        "version: 1\narchitecture: { layers: { web: [\"src/web/**\"] } }\nrules:\n  \
         forbidden_dependencies: [ { id: r1, from: web, to: persistance, reason: x } ]\n",
    );
    assert_eq!(loaded.status, ConfigStatus::Invalid);
    let issue = loaded.errors().next().unwrap();
    assert_eq!(issue.kind, ConfigIssueKind::UndefinedLayer);
    assert_eq!(issue.path, "rules.forbidden_dependencies[0].to");

    // Inferable role names need no declaration.
    let inferable = load(
        "version: 1\nrules:\n  forbidden_dependencies: [ { id: r1, from: controller, to: \
         repositories, reason: x } ]\n",
    );
    assert_eq!(
        inferable.status,
        ConfigStatus::Valid,
        "{:?}",
        inferable.issues
    );
}

#[test]
fn invalid_glob_rejected() {
    let loaded = load("version: 1\nignore: [\"src/[a\"]\n");
    assert_eq!(
        loaded.errors().next().unwrap().kind,
        ConfigIssueKind::InvalidGlob
    );
}

#[test]
fn config_hash_order_independent() {
    let a = load(
        "version: 1\nreview:\n  budgets: { max_symbols: 50 }\n  publish: { inline_cap: 10 }\n\
         ignore: [dist/**]\n",
    );
    let b = load(
        "# a comment\nignore:\n  - dist/**\nreview:\n  publish:\n    inline_cap: 10\n  \
         budgets:\n    max_symbols: 50\nversion: 1\n",
    );
    assert_eq!(a.status, ConfigStatus::Valid, "{:?}", a.issues);
    assert_eq!(a.config_hash, b.config_hash);
    assert_eq!(a.normalized, b.normalized);
    // Writing a default explicitly is the same config.
    let c = load(
        "version: 1\nignore: [dist/**]\nreview:\n  budgets: { max_symbols: 50, max_model_calls: \
         30 }\n  publish: { inline_cap: 10 }\n",
    );
    assert_eq!(a.config_hash, c.config_hash);
    let d = load("version: 1\nignore: [dist/**]\n");
    assert_ne!(a.config_hash, d.config_hash);
}

#[test]
fn yaml_bomb_rejected() {
    let bomb = r#"
version: 1
a: &a ["lol","lol","lol","lol","lol","lol","lol","lol","lol"]
b: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a]
c: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b]
d: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c]
e: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d]
f: &f [*e,*e,*e,*e,*e,*e,*e,*e,*e]
g: &g [*f,*f,*f,*f,*f,*f,*f,*f,*f]
h: &h [*g,*g,*g,*g,*g,*g,*g,*g,*g]
i: &i [*h,*h,*h,*h,*h,*h,*h,*h,*h]
"#;
    let loaded = load(bomb);
    assert_eq!(loaded.status, ConfigStatus::Invalid);
    assert_eq!(
        loaded.errors().next().unwrap().kind,
        ConfigIssueKind::YamlBomb
    );

    let deep = format!("version: 1\nx: {}1{}\n", "[".repeat(40), "]".repeat(40));
    let loaded = load(&deep);
    assert_eq!(loaded.status, ConfigStatus::Invalid);
    assert!(matches!(
        loaded.errors().next().unwrap().kind,
        ConfigIssueKind::YamlBomb
    ));

    let huge = format!("version: 1\n#{}\n", "x".repeat(300 * 1024));
    assert_eq!(
        load(&huge).errors().next().unwrap().kind,
        ConfigIssueKind::TooLarge
    );
}

#[test]
fn path_escape_rejected() {
    for text in [
        "version: 1\nknowledge_sources: [ { id: k, kind: adr, path: ../other-repo/docs } ]\n",
        "version: 1\nignore: [\"/etc/**\"]\n",
        "version: 1\nrules:\n  database: { migrations_only: true, migration_paths: [\"src/../../x/**\"] }\n",
    ] {
        let loaded = load(text);
        assert_eq!(loaded.status, ConfigStatus::Invalid, "{text}");
        assert_eq!(
            loaded.errors().next().unwrap().kind,
            ConfigIssueKind::PathEscape,
            "{text}"
        );
    }
}

#[test]
fn invalid_config_ignores_suppressions() {
    let loaded = load(
        "version: 1\nsuppressions: [ { id: s1, type: path, value: \"src/**\", reason: r } ]\n\
         review:\n  confidence: { minimum_publish: 0.1 }\n",
    );
    assert_eq!(loaded.status, ConfigStatus::Invalid);
    assert!(loaded.applicable_suppressions().is_empty());
    assert_eq!(loaded.config, ReviewConfigV1::default());
    assert!(!loaded.issues.is_empty());

    let valid = load(
        "version: 1\nsuppressions: [ { id: s1, type: path, value: \"src/**\", reason: r } ]\n",
    );
    assert_eq!(valid.applicable_suppressions().len(), 1);
}

#[test]
fn expired_suppression_is_a_warning() {
    let loaded = load(
        "version: 1\nsuppressions: [ { id: old, type: rule, value: r1, reason: r, expires: 2020-01-01 } ]\n",
    );
    assert_eq!(loaded.status, ConfigStatus::Valid);
    assert_eq!(loaded.issues[0].kind, ConfigIssueKind::ExpiredSuppression);
    assert!(!loaded.issues[0].is_error());
}

#[test]
fn suppression_requires_reason() {
    let loaded =
        load("version: 1\nsuppressions: [ { id: s, type: rule, value: r1, reason: \" \" } ]\n");
    assert_eq!(
        loaded.errors().next().unwrap().kind,
        ConfigIssueKind::MissingReason
    );
}

#[test]
fn init_template_validates() {
    let loaded = load(repository::review_dir::STARTER_CONFIG);
    assert_eq!(loaded.status, ConfigStatus::Valid, "{:?}", loaded.issues);
    assert!(loaded.issues.is_empty());
    // The starter is the defaults written out.
    assert_eq!(loaded.config, ReviewConfigV1::default());
}

#[test]
fn schema_is_closed_and_complete() {
    let schema = serde_json::to_value(profile::config::json_schema()).unwrap();
    assert_eq!(
        schema["additionalProperties"],
        serde_json::Value::Bool(false)
    );
    for key in [
        "version",
        "review",
        "architecture",
        "rules",
        "conventions",
        "suppressions",
        "knowledge_sources",
    ] {
        assert!(schema["properties"].get(key).is_some(), "{key}");
    }
}
