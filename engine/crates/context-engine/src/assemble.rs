//! Context assembly: candidates, ranking, gating, compression, budgeting and the semantic fill
//! combined into one sealed [`ContextPackage`] (target-architecture section 3.8).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use review_core::ids::SymbolKey;
use review_core::location::DiffSide;

use crate::budget::{
    estimate_tokens, fill_remaining, select, semantic_fill_needed, BudgetLimits,
    Candidate as BudgetCandidate,
};
use crate::candidates::api::ApiCandidates;
use crate::candidates::config::ConfigCandidates;
use crate::candidates::lexical::LexicalCandidates;
use crate::candidates::rules::RuleDocCandidates;
use crate::candidates::semantic::{self, SemanticSource};
use crate::candidates::structural::StructuralCandidates;
use crate::candidates::tests::TestCandidates;
use crate::candidates::{Candidate, CandidateGenerator, CandidatePool, SelectionInput};
use crate::compress::{self, BlobSource, Hints, Rendered};
use crate::hash::{seal, CONTEXT_ENGINE_VERSION};
use crate::model::{
    CandidateSource, ChangeSummary, ContextBudget, ContextItem, ContextPackage, ContextProvenance,
    ItemId, OmitReason, OmittedItem, RiskSignal, SemanticStatus,
};
use crate::ranking::{gate, score, GateOutcome, Weights, WEIGHTS_VERSION};

/// Lines of a symbol that drive its excerpt: changed lines and lines calling the seeds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineHints {
    pub changed_lines: Vec<u32>,
    pub seed_call_lines: Vec<u32>,
}

/// Everything assembly needs besides the candidates.
pub struct AssembleOptions<'a> {
    pub cluster_id: String,
    pub limits: BudgetLimits,
    pub weights: Weights,
    pub blobs: &'a dyn BlobSource,
    pub hints: BTreeMap<(SymbolKey, DiffSide), LineHints>,
    /// `None` skips the semantic fill.
    pub semantic: Option<&'a dyn SemanticSource>,
    /// Why semantic search is off (privacy mode, no collection); recorded in provenance.
    pub semantic_disabled: Option<String>,
    /// The summary template text of a seed, used as the semantic query.
    pub seed_summary: Option<&'a dyn Fn(SymbolKey) -> Option<String>>,
    pub semantic_space: Option<String>,
    pub risk_signals: Vec<RiskSignal>,
    pub change_summary: ChangeSummary,
    pub input_hashes: Vec<String>,
    pub flags: Vec<String>,
}

impl fmt::Debug for AssembleOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssembleOptions")
            .field("cluster_id", &self.cluster_id)
            .field("limits", &self.limits)
            .field("semantic", &self.semantic.is_some())
            .finish_non_exhaustive()
    }
}

/// Runs the generators in their fixed order: structural, tests, config, API, rule docs and
/// lexical. Returns the pool and provenance flags for degraded inputs.
pub fn generate(
    input: &SelectionInput<'_>,
    lexical: Option<&LexicalCandidates>,
) -> (CandidatePool, Vec<String>) {
    let mut pool = CandidatePool::new();
    let mut flags = Vec::new();
    let generators: [&dyn CandidateGenerator; 5] = [
        &StructuralCandidates,
        &TestCandidates,
        &ConfigCandidates,
        &ApiCandidates,
        &RuleDocCandidates,
    ];
    let mut structural_degraded = false;
    for g in generators {
        let stats = g.generate(input, &mut pool);
        if stats.degraded {
            match g.source() {
                crate::model::CandidateSource::RuleDoc => flags.push("profile_missing".to_owned()),
                _ => structural_degraded = true,
            }
        }
    }
    if structural_degraded {
        flags.push("structural_degraded".to_owned());
    }
    match lexical {
        Some(l) => {
            l.generate(input, &mut pool);
        }
        None => flags.push("lexical_disabled".to_owned()),
    }
    flags.sort();
    flags.dedup();
    (pool, flags)
}

enum Prepared {
    Ready(Box<BudgetCandidate>),
    Omit(OmittedItem),
}

fn omitted(cand: &Candidate, score: f32, reason: OmitReason) -> OmittedItem {
    OmittedItem {
        node: cand.node,
        title: cand.title.clone(),
        kind: cand.kind,
        score,
        reason,
    }
}

/// Scores, gates and renders one candidate into a budget candidate with its cheaper form.
fn prepare(input: &SelectionInput<'_>, cand: Candidate, opts: &AssembleOptions<'_>) -> Prepared {
    let signals = cand.raw.to_signals(cand.distance);
    let total = score(&signals, &opts.weights, cand.min_confidence, cand.weak);
    // Semantic-only candidates are fill by definition (budget and caps bound them), so the
    // distance gate, which needs two strong non-proximity signals at d3, does not apply.
    let semantic_only =
        cand.sources.len() == 1 && cand.sources.contains(&CandidateSource::Semantic);
    if !cand.pinned && !semantic_only {
        if let GateOutcome::Reject(reason) = gate(cand.distance, total, &signals, cand.kind) {
            return Prepared::Omit(omitted(&cand, total, reason));
        }
    }
    let commit = input.commit_for(cand.side);
    let empty = LineHints::default();
    let line_hints = cand
        .node
        .and_then(|n| opts.hints.get(&(n, cand.side)))
        .unwrap_or(&empty);
    let hints = Hints {
        changed_lines: &line_hints.changed_lines,
        seed_call_lines: &line_hints.seed_call_lines,
    };
    let level = compress::default_level(
        cand.kind,
        cand.distance,
        total,
        compress::span_lines(cand.span.as_ref()),
    );
    let rendered = compress::render(&cand, level, commit, opts.blobs, &hints);
    let Rendered::Content {
        content, locations, ..
    } = &rendered
    else {
        return Prepared::Omit(omitted(&cand, total, OmitReason::SecretBearing));
    };
    let tokens = rendered.tokens(&cand.title);
    let cheaper = compress::cheaper(level).and_then(|lower| {
        match compress::render(&cand, lower, commit, opts.blobs, &hints) {
            Rendered::Content { content, .. } => {
                let t = estimate_tokens(&format!("{}\n{}", cand.title, content.text()));
                (t < tokens).then_some((content, t))
            }
            Rendered::SecretBearing => None,
        }
    });
    let item = ContextItem {
        id: ItemId::of(cand.kind, cand.node.as_ref(), cand.side, &cand.title),
        kind: cand.kind,
        node: cand.node,
        side: cand.side,
        title: cand.title.clone(),
        relation: cand.relation.clone(),
        distance: cand.distance,
        path: cand.path.clone(),
        sources: cand.sources.clone(),
        signals,
        score: total,
        content: content.clone(),
        locations: locations.clone(),
        tokens,
        pinned: cand.pinned,
    };
    Prepared::Ready(Box::new(BudgetCandidate { item, cheaper }))
}

fn sort_prepared(
    input: &SelectionInput<'_>,
    cands: Vec<Candidate>,
    opts: &AssembleOptions<'_>,
    omitted_out: &mut Vec<OmittedItem>,
) -> Vec<BudgetCandidate> {
    let mut ready = Vec::new();
    for cand in cands {
        match prepare(input, cand, opts) {
            Prepared::Ready(b) => ready.push(*b),
            Prepared::Omit(o) => omitted_out.push(o),
        }
    }
    ready
}

/// Ranks, gates, compresses and budgets `pool` into a sealed package. Every candidate ends up
/// either in `items` or in `omitted` (capped, with the overflow counted) exactly once.
pub fn assemble(
    input: &SelectionInput<'_>,
    pool: CandidatePool,
    opts: &AssembleOptions<'_>,
) -> ContextPackage {
    let pool_nodes: BTreeSet<SymbolKey> = pool.iter().filter_map(|c| c.node).collect();
    let mut omitted_all: Vec<OmittedItem> = Vec::new();
    let ready = sort_prepared(input, pool.into_candidates(), opts, &mut omitted_all);
    let mut selection = select(ready, &opts.limits);

    let mut status = SemanticStatus::NotRun;
    if let Some(source) = opts.semantic {
        if semantic_fill_needed(&selection, &opts.limits) {
            let mut selected_nodes: BTreeSet<SymbolKey> =
                selection.selected.iter().filter_map(|i| i.node).collect();
            selected_nodes.extend(pool_nodes.iter().copied());
            let summaries = |key: SymbolKey| -> Option<String> {
                opts.seed_summary.and_then(|f| f(key)).or_else(|| {
                    input
                        .seeds
                        .iter()
                        .find(|s| s.key == key)
                        .map(|s| s.title.clone())
                })
            };
            let request = semantic::build_request(input, &summaries, &selected_nodes);
            match semantic::fetch(source, &request) {
                Ok(hits) => {
                    let cands = semantic::to_candidates(hits, &request);
                    let extra = sort_prepared(input, cands, opts, &mut omitted_all);
                    let before = selection.selected.len();
                    selection = fill_remaining(selection, extra, &opts.limits);
                    let count = selection.selected.len().saturating_sub(before);
                    status = SemanticStatus::Added {
                        count: u32::try_from(count).unwrap_or(u32::MAX),
                    };
                }
                Err(s) => status = s,
            }
        } else {
            status = SemanticStatus::NotNeeded {
                reason: "no remaining budget".to_owned(),
            };
        }
    } else if let Some(reason) = &opts.semantic_disabled {
        status = SemanticStatus::Unavailable {
            reason: reason.clone(),
        };
    }

    omitted_all.extend(selection.omitted);
    let l = &opts.limits;
    let max_items = u32::from(l.max_changed)
        + u32::from(l.max_related)
        + u32::from(l.max_tests)
        + u32::from(l.max_configs)
        + u32::from(l.max_rule_docs);
    let structural_degraded = opts.flags.iter().any(|f| f == "structural_degraded");
    let package = ContextPackage::new(
        input.reviewer,
        opts.cluster_id.clone(),
        selection.selected,
        omitted_all,
        ContextBudget {
            max_tokens: l.max_tokens,
            max_items,
        },
        opts.risk_signals.clone(),
        opts.change_summary.clone(),
        ContextProvenance {
            input_hashes: opts.input_hashes.clone(),
            engine_version: CONTEXT_ENGINE_VERSION.to_owned(),
            weights_version: WEIGHTS_VERSION.to_owned(),
            semantic_space: opts.semantic_space.clone(),
            structural_degraded,
            semantic: status,
            flags: opts.flags.clone(),
        },
    );
    seal(package)
}
