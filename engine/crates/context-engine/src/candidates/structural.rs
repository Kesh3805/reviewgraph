//! Structural candidates from the cluster's impact graph (CTX-002).

use std::collections::BTreeMap;

use impact::graph::{EntryKind, GraphSide, ImpactElement, SymbolImpact};
use impact::Relation;
use review_core::ids::SymbolKey;
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

use super::{
    convert_path, Candidate, CandidateGenerator, CandidatePool, ChangedSeed, GenStats, LineSpan,
    SelectionInput,
};
use crate::model::{CandidateSource, ItemKind};
use crate::ranking::structural_strength;

/// Weak elements become candidates only at or above this confidence.
pub const WEAK_MIN_CONFIDENCE: f32 = 0.3;

#[derive(Debug, Clone, Copy, Default)]
pub struct StructuralCandidates;

impl CandidateGenerator for StructuralCandidates {
    fn source(&self) -> CandidateSource {
        CandidateSource::Structural
    }

    fn generate(&self, input: &SelectionInput<'_>, out: &mut CandidatePool) -> GenStats {
        let mut stats = GenStats::default();
        for seed in input.seeds.iter().filter(|s| !s.skipped && !s.is_test) {
            for cand in seed_candidates(seed) {
                stats.record(out.insert(cand));
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
            let on_path = endpoint_path_nodes(impact);
            for el in &impact.elements {
                let Some(mut cand) = element_candidate(input, el, &on_path) else {
                    stats.skipped += 1;
                    continue;
                };
                if input.reviewer == ReviewerType::Security
                    && !cand.pinned
                    && !security_keep(input, impact.seed, el, &cand)
                {
                    stats.skipped += 1;
                    continue;
                }
                cand.raw.risk_importance = cand.raw.risk_importance.max(
                    input
                        .node_risk
                        .get(&el.node)
                        .copied()
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0),
                );
                stats.record(out.insert(cand));
            }
        }
        stats
    }
}

/// The changed symbol (both sides when its body changed) and its container's signature.
fn seed_candidates(seed: &ChangedSeed) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut changed = |side: DiffSide, start: u32, end: u32| {
        let mut c = Candidate::node(
            seed.key,
            seed.title.clone(),
            ItemKind::ChangedSymbol,
            side,
            CandidateSource::Structural,
        );
        c.distance = Some(0);
        c.pinned = true;
        c.raw.risk_importance = seed.risk.clamp(0.0, 1.0);
        c.span = Some(LineSpan {
            file: seed.file.clone(),
            start_line: start,
            end_line: end,
            side,
        });
        out.push(c);
    };
    changed(seed.side, seed.start_line, seed.end_line);
    if seed.body_changed && !seed.removed {
        let (start, end) = seed.base_range.unwrap_or((seed.start_line, seed.end_line));
        changed(DiffSide::Base, start, end);
    }
    if let Some(container) = &seed.container {
        let side = if seed.removed {
            DiffSide::Base
        } else {
            DiffSide::Head
        };
        let mut c = Candidate::node(
            container.key,
            container.title.clone(),
            ItemKind::ClassSignature,
            side,
            CandidateSource::Structural,
        );
        c.distance = Some(0);
        c.pinned = true;
        c.relation = Some(Relation::Container.as_str().to_owned());
        c.raw.structural = structural_strength(Relation::Container.as_str());
        c.span = Some(LineSpan {
            file: seed.file.clone(),
            start_line: container.start_line,
            end_line: container.end_line,
            side,
        });
        out.push(c);
    }
    out
}

/// Nodes on a path from the seed to an entrypoint, with `1.0` (HTTP) or `0.6` (queue, cron,
/// CLI, worker) scaled by the path's minimum confidence.
fn endpoint_path_nodes(impact: &SymbolImpact) -> BTreeMap<SymbolKey, f32> {
    let mut out: BTreeMap<SymbolKey, f32> = BTreeMap::new();
    for el in impact.of(Relation::Endpoint) {
        let base = match el.endpoint.as_ref().map(|e| e.entry_kind) {
            Some(EntryKind::Http) => 1.0,
            _ => 0.6,
        };
        let value = base * el.min_confidence.as_f32();
        let mut bump = |node: SymbolKey| {
            let slot = out.entry(node).or_insert(0.0);
            *slot = slot.max(value);
        };
        bump(el.node);
        for step in &el.path {
            bump(step.from);
            bump(step.to);
        }
    }
    out
}

fn element_side(el: &ImpactElement) -> DiffSide {
    if el.relation == Relation::RemovedCallee || el.missing_on_head {
        return DiffSide::Base;
    }
    match el.path.last().map(|s| s.graph) {
        Some(GraphSide::Base) => DiffSide::Base,
        _ => DiffSide::Head,
    }
}

/// `None` for relations other generators own and for weak elements below the floor.
fn element_candidate(
    input: &SelectionInput<'_>,
    el: &ImpactElement,
    on_path: &BTreeMap<SymbolKey, f32>,
) -> Option<Candidate> {
    if matches!(
        el.relation,
        Relation::Test | Relation::Config | Relation::EnvVar | Relation::Endpoint
    ) {
        return None;
    }
    let min_conf = el.min_confidence.as_f32();
    if el.weak && min_conf < WEAK_MIN_CONFIDENCE {
        return None;
    }
    let side = element_side(el);
    let kind = if el.relation == Relation::Container {
        ItemKind::ClassSignature
    } else {
        ItemKind::RelatedSymbol
    };
    let info = input.locator.and_then(|l| l.locate(el.node, side));
    let title = info
        .as_ref()
        .map_or_else(|| el.node_id.clone(), |i| i.title.clone());
    let mut c = Candidate::node(el.node, title, kind, side, CandidateSource::Structural);
    c.distance = Some(el.distance);
    c.path = convert_path(&el.path);
    c.min_confidence = min_conf;
    c.relation = Some(el.relation.as_str().to_owned());
    c.weak = el.weak;
    c.pinned = kind == ItemKind::ClassSignature;
    c.raw.structural = structural_strength(el.relation.as_str());
    c.raw.execution_path = on_path.get(&el.node).copied().unwrap_or(0.0);
    c.span = info.map(|i| LineSpan {
        file: i.file,
        start_line: i.start_line,
        end_line: i.end_line,
        side,
    });
    Some(c)
}

/// Trust-boundary filter for the security reviewer: keep candidates on a path to an entrypoint,
/// interface/implementation/override relatives of an authorization symbol, authorization,
/// validation or authentication nodes, and removed callees.
fn security_keep(
    input: &SelectionInput<'_>,
    seed: SymbolKey,
    el: &ImpactElement,
    cand: &Candidate,
) -> bool {
    el.relation == Relation::RemovedCallee
        || cand.raw.execution_path > 0.0
        || input.trust_nodes.contains(&el.node)
        || (matches!(
            el.relation,
            Relation::Interface | Relation::Implementation | Relation::Override
        ) && input.trust_nodes.contains(&seed))
}
