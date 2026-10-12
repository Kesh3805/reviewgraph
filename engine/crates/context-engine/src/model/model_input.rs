//! Mapping of a package onto the PRD section 89 model input shape.

use serde_json::{json, Value};

use super::item::{ContextItem, ItemKind};
use super::package::ContextPackage;

fn excerpt(item: &ContextItem) -> Value {
    let location = item
        .locations
        .first()
        .map(|l| l.render())
        .unwrap_or_default();
    json!({
        "title": item.title,
        "kind": item.kind.as_str(),
        "relation": item.relation,
        "distance": item.distance,
        "location": location,
        "code": item.content.text(),
    })
}

impl ContextPackage {
    /// `changedSymbols, changeSummary, graphNeighborhood, tests, repositoryRules, riskSignals,
    /// deterministicFindings`. Every excerpt carries `"location": "path:start-end@side"`.
    pub fn to_model_input(&self) -> Value {
        let pick = |f: fn(ItemKind) -> bool| -> Vec<Value> {
            self.items
                .iter()
                .filter(|i| f(i.kind))
                .map(excerpt)
                .collect()
        };
        json!({
            "changedSymbols": pick(|k| matches!(k, ItemKind::ChangedSymbol | ItemKind::ClassSignature)),
            "changeSummary": self.change_summary,
            "graphNeighborhood": pick(|k| matches!(
                k,
                ItemKind::RelatedSymbol | ItemKind::ApiEndpoint | ItemKind::Config
                    | ItemKind::SchemaChange | ItemKind::DependencyChange
            )),
            "tests": pick(|k| k == ItemKind::Test),
            "repositoryRules": pick(|k| matches!(k, ItemKind::RuleDoc | ItemKind::Convention)),
            "riskSignals": self.risk_signals,
            "deterministicFindings": [],
        })
    }
}
