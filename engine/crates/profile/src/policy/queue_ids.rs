//! `rules.queue_jobs.require_deterministic_id` (POL-003): a new or changed job-producing call
//! whose `jobId` is missing or random (the PROF-005 predicate). A call that already had the same
//! job id expression in the base is pre-existing and not reported.

use std::collections::{BTreeMap, BTreeSet};

use codegraph::{Direction, EdgeKind};
use review_core::evidence::EvidenceKind;

use super::change::SymbolChange;
use super::evaluator::{FileRange, RuleContext, RuleEvaluator, RuleOutcome, RuleViolation};
use super::evidence::{deterministic, location, symbol_ref};
use super::templates::render;
use crate::config::{ReviewConfigV1, RuleSeverity};
use crate::conventions::builtin::edges;
use crate::conventions::builtin::queues::{is_deterministic_job_id, is_job_call};
use crate::facts::{detail_str, fact_line, FactIndex};

pub const RULE_KEY: &str = "queue_deterministic_job_id";
pub const RULE_ID: &str = "rules.queue_jobs.require_deterministic_id";

#[derive(Debug, Clone, Copy, Default)]
pub struct QueueDeterministicIds;

/// The job id expressions (`""` for a missing one) of a symbol's job calls.
fn job_ids(facts: &FactIndex, key: codegraph::NodeKey) -> BTreeSet<String> {
    facts
        .facts(key)
        .iter()
        .filter(|f| is_job_call(f))
        .map(|f| detail_str(f, "job_id").unwrap_or_default().trim().to_owned())
        .collect()
}

impl RuleEvaluator for QueueDeterministicIds {
    fn rule_key(&self) -> &'static str {
        RULE_KEY
    }

    fn enabled(&self, config: &ReviewConfigV1) -> bool {
        config.rules.queue_jobs.require_deterministic_id
    }

    fn evaluate(&self, cx: &RuleContext<'_>) -> RuleOutcome {
        let head = cx.graphs.head;
        let facts = cx.graphs.head_facts;
        let producers: Vec<_> = cx
            .change
            .symbols
            .iter()
            .filter(|s| s.change != SymbolChange::Removed)
            .filter(|s| !edges(head, s.key, Direction::Out, &[EdgeKind::ProducesJob]).is_empty())
            .collect();
        if !producers.is_empty() && producers.iter().all(|s| facts.facts(s.key).is_empty()) {
            return RuleOutcome::not_executed(
                RULE_KEY,
                "syntax facts are unavailable for the changed job producers",
            );
        }
        let mut violations = Vec::new();
        for symbol in producers {
            let Some(node) = head.node(symbol.key) else {
                continue;
            };
            let before = job_ids(cx.graphs.base_facts, symbol.key);
            for fact in facts.facts(symbol.key).iter().filter(|f| is_job_call(f)) {
                let job_id = detail_str(fact, "job_id").unwrap_or_default().trim().to_owned();
                if is_deterministic_job_id(&job_id) || before.contains(&job_id) {
                    continue;
                }
                let problem = if job_id.is_empty() {
                    "without a jobId".to_owned()
                } else {
                    format!("with the random jobId `{job_id}`")
                };
                let line = fact_line(fact);
                let mut params = BTreeMap::new();
                params.insert("rule_id".to_owned(), RULE_ID.to_owned());
                params.insert("symbol".to_owned(), node.qualified_name.to_owned());
                params.insert("problem".to_owned(), problem);
                params.insert("job_id".to_owned(), job_id.clone());
                let message = render(RULE_KEY, &params);
                let evidence = deterministic(
                    "policy:queue_deterministic_job_id",
                    EvidenceKind::StaticAnalysisResult,
                    &message,
                    location(&symbol.path, line, line),
                    symbol_ref(node.id, node.qualified_name).into_iter().collect(),
                )
                .into_iter()
                .collect();
                violations.push(RuleViolation {
                    rule_id: RULE_ID.to_owned(),
                    rule_key: RULE_KEY.to_owned(),
                    anchor: FileRange {
                        path: symbol.path.clone(),
                        start_line: line,
                        end_line: line,
                    },
                    symbol_key: Some(node.id.to_owned()),
                    evidence,
                    severity: RuleSeverity::High,
                    params,
                    message,
                });
            }
        }
        RuleOutcome::executed(RULE_KEY, violations)
    }
}
