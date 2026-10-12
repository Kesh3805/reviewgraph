//! `rules.forbidden_dependencies` (POL-003): delegates to the REV-A-002 boundary check with the
//! explicit rules only (conventions are the architecture reviewer's business, not policy).

use std::collections::BTreeMap;

use super::evaluator::{FileRange, RuleContext, RuleEvaluator, RuleOutcome, RuleViolation};
use super::layers::LayerMap;
use super::templates::render;
use crate::config::ReviewConfigV1;
use crate::prechecks::boundaries::{check_boundaries, forbidden_pairs};
use crate::prechecks::PrecheckStatus;

pub const RULE_KEY: &str = "forbidden_dependency";

#[derive(Debug, Clone, Copy, Default)]
pub struct ForbiddenDependencies;

impl RuleEvaluator for ForbiddenDependencies {
    fn rule_key(&self) -> &'static str {
        RULE_KEY
    }

    fn enabled(&self, config: &ReviewConfigV1) -> bool {
        !config.rules.forbidden_dependencies.is_empty()
    }

    fn evaluate(&self, cx: &RuleContext<'_>) -> RuleOutcome {
        let layers = LayerMap::from_graph(cx.config, cx.graphs.head);
        let pairs = forbidden_pairs(cx.config, None);
        let undefined: Vec<String> = pairs
            .iter()
            .flat_map(|p| [&p.from, &p.to])
            .filter(|layer| !layers.defines(layer))
            .map(|l| l.to_string())
            .collect();
        if undefined.len() == pairs.len() * 2 {
            return RuleOutcome::not_executed(
                RULE_KEY,
                format!(
                    "no file belongs to the layers {} (neither declared nor inferred)",
                    undefined.join(", ")
                ),
            );
        }
        let outcome = check_boundaries(cx.change, &cx.graphs, &layers, &pairs);
        if let PrecheckStatus::NotExecuted { reason } = outcome.status {
            return RuleOutcome::not_executed(RULE_KEY, reason);
        }
        let violations = outcome
            .findings
            .into_iter()
            .filter_map(|finding| {
                let rule = pairs
                    .iter()
                    .find(|p| Some(&p.id) == finding.basis_id.as_ref())?;
                let (from_symbol, to_symbol, import) = match finding.details.first() {
                    Some(crate::prechecks::PrecheckEvidence::Edge { from, to, kind, .. }) => {
                        (from.clone(), to.clone(), kind.clone())
                    }
                    _ => (String::new(), String::new(), String::new()),
                };
                let mut params = BTreeMap::new();
                params.insert("rule_id".to_owned(), rule.id.clone());
                params.insert("from".to_owned(), rule.from.clone());
                params.insert("to".to_owned(), rule.to.clone());
                params.insert("from_symbol".to_owned(), from_symbol);
                params.insert("to_symbol".to_owned(), to_symbol);
                params.insert("import".to_owned(), import);
                params.insert("reason".to_owned(), rule.reason.clone());
                if let Some(fix) = &rule.fix {
                    params.insert("fix".to_owned(), fix.clone());
                }
                Some(RuleViolation {
                    rule_id: rule.id.clone(),
                    rule_key: RULE_KEY.to_owned(),
                    anchor: FileRange {
                        path: finding.path.clone(),
                        start_line: finding.line,
                        end_line: finding.line,
                    },
                    symbol_key: finding.symbol_key.clone(),
                    evidence: finding.evidence,
                    severity: rule.severity,
                    message: render(RULE_KEY, &params),
                    params,
                })
            })
            .collect();
        RuleOutcome::executed(RULE_KEY, violations)
    }
}
