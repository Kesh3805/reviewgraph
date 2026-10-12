//! The test reviewer (REV-T-001): routing predicate, budget, the `tests` prompt binding, the
//! `tests.v1` schema and candidate extension, and the stale-mock hint.
//!
//! Evidence from the REV-T-002 pre-check reaches the model through the context's
//! `deterministic_findings` section. Every finding cites a mapped test (`test_case_ref`) or
//! states `no_mapped_test` with that pre-check evidence (`mapping_evidence_ref`); the
//! [`crate::gates`] drop anything else.

use std::sync::LazyLock;

use model_gateway::{InputSection, OutputSchema, TaskType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::ReviewContext;
use crate::error::ReviewerError;
use crate::prompts::{prompt, Prompt};
use crate::reviewer::{
    Applicability, ContextBudget, ReviewRequest, ReviewerKind, RiskAssessment, RoutingInput,
};
use crate::routing::SkipReason;
use crate::specialist::{always_ready, any_signal, SpecialistReviewer, SpecialistSpec};

/// Semver of the test reviewer code.
pub const TESTS_REVIEWER_VERSION: &str = "1.1.0";

/// Minimum confidence to publish a test finding (default threshold override).
pub const TESTS_MIN_PUBLISH_CONFIDENCE: f64 = 0.80;

pub const TESTS_SCHEMA_NAME: &str = "tests";
pub const TESTS_SCHEMA_VERSION: &str = "v1";
pub const TESTS_SCHEMA_JSON: &str = include_str!("../prompts/tests/v1.schema.json");

/// The seven test-gap categories of `tests.v1`.
pub const TESTS_CATEGORIES: &[&str] = &[
    "changed_behavior_untested",
    "assertion_no_longer_covers",
    "stale_mock",
    "missing_negative_test",
    "missing_concurrency_test",
    "broken_fixture",
    "implementation_coupled_test",
];

/// Behavioural change classes (CHG-002..005 and the clustering of IMP-009) that make the test
/// reviewer apply.
pub const TESTS_BEHAVIOUR_CLASSES: &[&str] = &[
    "condition_changed",
    "return_changed",
    "exception_changed",
    "throw_changed",
    "call_added",
    "call_removed",
    "signature_changed",
    "exported_signature_changed",
    "body_changed",
    "attrs_changed",
    "symbol_added",
    "symbol_removed",
];

static TESTS_SCHEMA: LazyLock<Result<OutputSchema, String>> = LazyLock::new(|| {
    serde_json::from_str::<Value>(TESTS_SCHEMA_JSON)
        .map(|v| OutputSchema::new(TESTS_SCHEMA_NAME, TESTS_SCHEMA_VERSION, v))
        .map_err(|e| e.to_string())
});

/// The `tests.v1` output schema.
pub fn tests_output_schema() -> Result<OutputSchema, ReviewerError> {
    TESTS_SCHEMA
        .clone()
        .map_err(|e| ReviewerError::Prompt(format!("tests.v1 schema is not valid JSON: {e}")))
}

/// The test reviewer prompt, v2 (binds `tests.v1`).
pub fn tests_prompt() -> Result<Prompt, ReviewerError> {
    prompt("tests", 2)
}

/// The fields `tests.v1` adds to the common candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCandidateExtension {
    pub category: String,
    /// `T#` ref of the test that should cover the behaviour.
    pub test_case_ref: Option<String>,
    pub no_mapped_test: bool,
    /// `D#` ref of the REV-T-002 evidence that no test maps.
    pub mapping_evidence_ref: Option<String>,
}

impl TestCandidateExtension {
    /// Reads the extension from one raw `tests.v1` finding.
    pub fn from_raw(finding: &Value) -> Option<Self> {
        serde_json::from_value(json!({
            "category": finding.get("category")?,
            "test_case_ref": finding.get("test_case_ref")?,
            "no_mapped_test": finding.get("no_mapped_test")?,
            "mapping_evidence_ref": finding.get("mapping_evidence_ref")?,
        }))
        .ok()
    }
}

/// Applies when a behavioural change class is present; skips tests-only, rename-only and
/// formatting-only clusters.
pub fn tests_applies(input: &RoutingInput<'_>) -> Applicability {
    if any_signal(input, TESTS_BEHAVIOUR_CLASSES) && !input.cluster.signals.contains("tests_only") {
        Applicability::Applies {
            focus_profiles: Vec::new(),
        }
    } else {
        Applicability::Skip {
            reason: SkipReason::NotApplicable,
        }
    }
}

/// Token budget by risk. The context engine caps the mapped tests at four, ranked by `TESTS`
/// edge confidence.
pub fn tests_budget(risk: &RiskAssessment) -> ContextBudget {
    ContextBudget::for_risk(risk.level)
}

/// The last name segment of a `SymbolId` text: `ts:src/a#Cls.method/method` gives `method`.
fn symbol_name(symbol_id: &str) -> &str {
    let after_module = symbol_id
        .rsplit_once('#')
        .map_or(symbol_id, |(_, rest)| rest);
    let qualified = after_module
        .rsplit_once('/')
        .map_or(after_module, |(name, _)| name);
    qualified.rsplit('.').next().unwrap_or(qualified)
}

const MOCK_MARKERS: &[&str] = &[
    "mockResolvedValue",
    "mockReturnValue",
    "mockImplementation",
    "useValue",
    "jest.mock",
];

/// Stale-mock hints: a changed symbol whose signature differs base to head while a mock fact in
/// the context still stubs it. The mock's value shape is unchanged because the mock item is not
/// a changed symbol; the model decides whether the stub no longer fits the new signature.
pub fn stale_mock_hints(cx: &dyn ReviewContext) -> Vec<Value> {
    let mut hints = Vec::new();
    for (i, s) in cx.changed_symbols().iter().enumerate() {
        let (Some(base), Some(head)) = (&s.signature_base, &s.signature_head) else {
            continue;
        };
        if base == head {
            continue;
        }
        let name = symbol_name(&s.symbol_id);
        if name.is_empty() {
            continue;
        }
        for (j, item) in cx.config_items().iter().enumerate() {
            let is_mock = MOCK_MARKERS.iter().any(|m| item.excerpt.contains(m));
            if is_mock && item.excerpt.contains(name) {
                hints.push(json!({
                    "symbol_ref": format!("S{}", i + 1),
                    "mock_ref": format!("C{}", j + 1),
                    "signature_changed": true,
                    "mock_shape_unchanged": true,
                }));
            }
        }
    }
    hints
}

fn tests_sections(req: &ReviewRequest<'_>) -> Vec<InputSection> {
    let hints = stale_mock_hints(req.context);
    if hints.is_empty() {
        Vec::new()
    } else {
        vec![InputSection::new("stale_mock_hints", Value::Array(hints))]
    }
}

/// Builds the test reviewer (prompt `tests/v2`, schema `tests.v1`).
pub fn tests_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Test,
            task: TaskType::TestReview,
            version: TESTS_REVIEWER_VERSION,
            applies: tests_applies,
            extra: tests_sections,
            ready: always_ready,
            budget: tests_budget,
        },
        tests_prompt()?,
        tests_output_schema()?,
    ))
}
