//! `rules.tests.public_api_changes_require_tests` (POL-003): a changed API endpoint contract
//! (route, DTO or response type) with no mapped test added or changed in the pull request. The
//! test mapping is REV-T-002's: a touched test file covers the handler when it imports the
//! handler's file, tests it, or names it in a suite or case title.

use std::collections::{BTreeMap, BTreeSet};

use review_core::evidence::EvidenceKind;

use super::change::FileStatus;
use super::evaluator::{FileRange, RuleContext, RuleEvaluator, RuleOutcome, RuleViolation};
use super::evidence::{deterministic, location, symbol_ref};
use super::templates::render;
use crate::config::{ReviewConfigV1, RuleSeverity};
use crate::infer::roles::is_test_path;
use crate::prechecks::tests_gap::covered_by_touched_tests;

pub const RULE_KEY: &str = "public_api_changes_require_tests";
pub const RULE_ID: &str = "rules.tests.public_api_changes_require_tests";

#[derive(Debug, Clone, Copy, Default)]
pub struct ApiChangesRequireTests;

impl RuleEvaluator for ApiChangesRequireTests {
    fn rule_key(&self) -> &'static str {
        RULE_KEY
    }

    fn enabled(&self, config: &ReviewConfigV1) -> bool {
        config.rules.tests.public_api_changes_require_tests
    }

    fn evaluate(&self, cx: &RuleContext<'_>) -> RuleOutcome {
        if !cx.test_framework_detected {
            return RuleOutcome::not_executed(
                RULE_KEY,
                "no test framework detected; tests cannot be mapped to endpoints",
            );
        }
        let head = cx.graphs.head;
        let touched_tests: BTreeSet<&str> = cx
            .change
            .files
            .iter()
            .filter(|f| f.status != FileStatus::Deleted && is_test_path(&f.path))
            .map(|f| f.path.as_str())
            .collect();
        let mut violations = Vec::new();
        for api in &cx.change.api_contract_changes {
            let handler = api.handler.and_then(|k| head.node(k));
            let covered = handler
                .as_ref()
                .is_some_and(|h| covered_by_touched_tests(head, &touched_tests, h));
            if covered {
                continue;
            }
            let mut params = BTreeMap::new();
            params.insert("rule_id".to_owned(), RULE_ID.to_owned());
            params.insert("endpoint".to_owned(), api.endpoint_id.clone());
            params.insert("what".to_owned(), api.what.clone());
            let message = render(RULE_KEY, &params);
            let symbols = handler
                .as_ref()
                .and_then(|h| symbol_ref(h.id, h.qualified_name))
                .into_iter()
                .collect();
            let evidence = deterministic(
                "policy:public_api_changes_require_tests",
                EvidenceKind::InterfaceContract,
                &message,
                location(&api.path, api.line, api.line),
                symbols,
            )
            .into_iter()
            .collect();
            violations.push(RuleViolation {
                rule_id: RULE_ID.to_owned(),
                rule_key: RULE_KEY.to_owned(),
                anchor: FileRange {
                    path: api.path.clone(),
                    start_line: api.line,
                    end_line: api.line,
                },
                symbol_key: handler.map(|h| h.id.to_owned()),
                evidence,
                severity: RuleSeverity::Medium,
                params,
                message,
            });
        }
        RuleOutcome::executed(RULE_KEY, violations)
    }
}
