//! Explicit repository policy: rule evaluation (POL-003), precedence (POL-004), violations as
//! deterministic evidence (POL-005) and suppressions (POL-006).

pub mod api_tests;
pub mod change;
pub mod evaluator;
pub mod evidence;
pub mod forbidden_deps;
pub mod layers;
pub mod migrations_only;
pub mod precedence;
pub mod queue_ids;
pub mod suppression;
pub mod templates;
pub mod to_candidates;

use rayon::prelude::*;

pub use change::{ChangeSet, GraphPair};
pub use evaluator::{RuleContext, RuleEvaluator, RuleOutcome, RuleStatus, RuleViolation};
pub use precedence::{Decision, EffectivePolicy, FileOrSymbol, PolicyResolver, PolicySource, PolicyTopic};

/// The four built-in evaluators.
pub fn evaluators() -> Vec<Box<dyn RuleEvaluator>> {
    vec![
        Box::new(forbidden_deps::ForbiddenDependencies),
        Box::new(migrations_only::MigrationsOnly),
        Box::new(api_tests::ApiChangesRequireTests),
        Box::new(queue_ids::QueueDeterministicIds),
    ]
}

/// Runs every enabled evaluator in parallel. Outcomes come back in evaluator order.
pub fn evaluate_rules(cx: &RuleContext<'_>) -> Vec<RuleOutcome> {
    evaluators()
        .par_iter()
        .filter(|e| e.enabled(cx.config))
        .map(|e| {
            let outcome = e.evaluate(cx);
            let status = if outcome.is_executed() {
                "executed"
            } else {
                "not_executed"
            };
            crate::metrics::rule_evaluation(e.rule_key(), status);
            crate::metrics::rule_violations(e.rule_key(), outcome.violations.len());
            outcome
        })
        .collect()
}
