//! Mapping of a package onto the PRD section 89 model input shape.

use serde_json::{json, Value};

use super::item::{ContextItem, ItemContent, ItemKind};
use super::package::ContextPackage;

fn excerpt(item: &ContextItem) -> Value {
    let location = item
        .locations
        .first()
        .map(|l| l.render())
        .unwrap_or_default();
    let locations: Vec<String> = item.locations.iter().map(|l| l.render()).collect();
    let structured = match &item.content {
        ItemContent::Structured { value } => value.clone(),
        _ => Value::Null,
    };
    json!({
        "title": item.title,
        "kind": item.kind.as_str(),
        "relation": item.relation,
        "distance": item.distance,
        "location": location,
        "locations": locations,
        "code": item.content.text().into_owned(),
        "structured": structured,
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
