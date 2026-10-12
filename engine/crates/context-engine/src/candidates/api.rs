//! API candidates (CTX-003): reached endpoints with their guards and DTO types, the handler of
//! a changed contract, and the DTO classes it accepts or returns.

use std::collections::BTreeSet;

use impact::Relation;
use review_core::ids::SymbolKey;
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;
use serde_json::json;

use super::{
    convert_path, Candidate, CandidateGenerator, CandidatePool, GenStats, LineSpan, SelectionInput,
};
use crate::model::{CandidateSource, ItemKind};

/// A changed API contract (CHG-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiChange {
    /// `PUT /users/:id`.
    pub endpoint: String,
    pub handler: Option<SymbolKey>,
    /// DTO and related types of the contract.
    pub related: Vec<SymbolKey>,
    pub breaking: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ApiCandidates;

impl CandidateGenerator for ApiCandidates {
    fn source(&self) -> CandidateSource {
        CandidateSource::Api
    }

    fn generate(&self, input: &SelectionInput<'_>, out: &mut CandidatePool) -> GenStats {
        let mut stats = GenStats::default();
        let changed_nodes: BTreeSet<SymbolKey> = input
            .api_changes
            .iter()
            .flat_map(|a| a.handler.iter().chain(a.related.iter()).copied())
            .collect();
        let title_of = |node: SymbolKey| -> String {
            input
                .locator
                .and_then(|l| l.locate(node, DiffSide::Head))
                .map_or_else(|| node.to_string(), |i| i.title)
        };
        for change in &input.api_changes {
            let dtos: Vec<String> = change.related.iter().map(|k| title_of(*k)).collect();
            let nodes = change.handler.iter().chain(change.related.iter());
            for node in nodes {
                let mut c = Candidate::node(
                    *node,
                    title_of(*node),
                    ItemKind::RelatedSymbol,
                    DiffSide::Head,
                    CandidateSource::Api,
                );
                c.distance = Some(1);
                c.raw.api_relationship = 1.0;
                c.relation = Some("api_contract".to_owned());
                c.structured = Some(json!({
                    "endpoint": change.endpoint,
                    "breaking": change.breaking,
                    "dto_types": dtos,
                }));
                if let Some(info) = input.locator.and_then(|l| l.locate(*node, DiffSide::Head)) {
                    c.span = Some(LineSpan {
                        file: info.file,
                        start_line: info.start_line,
                        end_line: info.end_line,
                        side: DiffSide::Head,
                    });
                }
                stats.record(out.insert(c));
            }
        }
        let Some(graph) = input.impact else {
            stats.degraded = true;
            return stats;
        };
        let seeds = input.seed_keys();
        for impact in graph
            .symbols
            .iter()
            .filter(|i| seeds.contains(&i.seed) && i.skipped.is_none())
        {
            for el in impact.of(Relation::Endpoint) {
                let attrs = el.endpoint.as_ref();
                let title = match attrs {
                    Some(a) => format!(
                        "http:{} {}",
                        a.method.as_deref().unwrap_or("-"),
                        a.path.as_deref().unwrap_or("-")
                    ),
                    None => el.node_id.clone(),
                };
                let mut c = Candidate::node(
                    el.node,
                    title,
                    ItemKind::ApiEndpoint,
                    DiffSide::Head,
                    CandidateSource::Api,
                );
                c.distance = Some(el.distance);
                c.path = convert_path(&el.path);
                c.min_confidence = el.min_confidence.as_f32();
                c.relation = Some(Relation::Endpoint.as_str().to_owned());
                c.weak = el.weak;
                c.pinned = input.reviewer == ReviewerType::Security;
                // A changed contract's own endpoint: 1.0; an endpoint merely reaching the change: 0.5.
                c.raw.api_relationship = if changed_nodes.contains(&el.node) {
                    1.0
                } else {
                    0.5
                };
                c.raw.execution_path = el.min_confidence.as_f32();
                c.structured = attrs.map(|a| {
                    let signature = format!(
                        "{} {}",
                        a.method.as_deref().unwrap_or("-"),
                        a.path.as_deref().unwrap_or("-")
                    );
                    let dtos: Vec<String> = input
                        .api_changes
                        .iter()
                        .filter(|change| change.endpoint == signature)
                        .flat_map(|change| change.related.iter().map(|k| title_of(*k)))
                        .collect();
                    json!({
                        "method": a.method,
                        "path": a.path,
                        "guards": a.guards,
                        "entry_kind": format!("{:?}", a.entry_kind).to_ascii_lowercase(),
                        "dto_types": dtos,
                    })
                });
                if let Some(info) = input
                    .locator
                    .and_then(|l| l.locate(el.node, DiffSide::Head))
                {
                    c.span = Some(LineSpan {
                        file: info.file,
                        start_line: info.start_line,
                        end_line: info.end_line,
                        side: DiffSide::Head,
                    });
                }
                stats.record(out.insert(c));
            }
        }
        stats
    }
}
