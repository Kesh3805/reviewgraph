//! Semantic candidates (CTX-008): used only to fill the budget that structural, test, config,
//! rule and lexical selection left over. They are never impact and never evidence.
//!
//! The vector search lives behind [`SemanticSource`]; the composition root implements it over the
//! tenant-scoped index (the tenant scope is fixed when the source is built) and enforces the
//! deadline. A failed or late search degrades to structural-only and says so in provenance.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use review_core::ids::SymbolKey;
use review_core::location::DiffSide;

use super::{Candidate, LineSpan, SelectionInput};
use crate::model::{CandidateSource, ItemKind, SemanticStatus};
use crate::ranking::semantic_signal;

/// Hits below this cosine similarity are ignored.
pub const MIN_COSINE: f32 = 0.55;
/// Most hits requested per query.
pub const MAX_HITS: usize = 20;
/// Seeds (highest risk first) that contribute a query text.
pub const MAX_QUERY_SEEDS: usize = 3;
/// Total time allowed for the search.
pub const SEARCH_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticRequest {
    /// Summary template text of each query seed's head version.
    pub query_texts: Vec<String>,
    /// Seeds and already selected nodes, never returned.
    pub exclude: BTreeSet<SymbolKey>,
    pub limit: usize,
    pub min_score: f32,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticHit {
    pub node: SymbolKey,
    pub title: String,
    /// Cosine similarity of the hit.
    pub cosine: f32,
    pub span: Option<LineSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SemanticUnavailable {
    #[error("semantic search timed out")]
    Timeout,
    #[error("semantic search failed: {0}")]
    Failed(String),
    #[error("semantic search is disabled: {0}")]
    Disabled(String),
}

pub trait SemanticSource {
    /// Searches within the tenant scope the source was built with.
    fn search(&self, request: &SemanticRequest) -> Result<Vec<SemanticHit>, SemanticUnavailable>;
}

/// Builds the request: the top seeds by risk, excluding the seeds and `selected`.
pub fn build_request(
    input: &SelectionInput<'_>,
    summaries: &dyn Fn(SymbolKey) -> Option<String>,
    selected: &BTreeSet<SymbolKey>,
) -> SemanticRequest {
    let mut seeds: Vec<_> = input.seeds.iter().filter(|s| !s.skipped).collect();
    seeds.sort_by(|a, b| b.risk.total_cmp(&a.risk).then_with(|| a.key.cmp(&b.key)));
    let query_texts = seeds
        .iter()
        .take(MAX_QUERY_SEEDS)
        .filter_map(|s| summaries(s.key))
        .collect();
    let mut exclude = input.seed_keys();
    exclude.extend(selected.iter().copied());
    SemanticRequest {
        query_texts,
        exclude,
        limit: MAX_HITS,
        min_score: MIN_COSINE,
        timeout: SEARCH_TIMEOUT,
    }
}

/// Runs the search under the request's timeout. A late answer counts as a timeout.
pub fn fetch(
    source: &dyn SemanticSource,
    request: &SemanticRequest,
) -> Result<Vec<SemanticHit>, SemanticStatus> {
    if request.query_texts.is_empty() {
        return Err(SemanticStatus::NotNeeded {
            reason: "no query seeds".to_owned(),
        });
    }
    let started = Instant::now();
    let hits = source
        .search(request)
        .map_err(|e| SemanticStatus::Unavailable {
            reason: e.to_string(),
        })?;
    if started.elapsed() > request.timeout {
        return Err(SemanticStatus::Unavailable {
            reason: SemanticUnavailable::Timeout.to_string(),
        });
    }
    Ok(hits)
}

/// Hits above the cosine floor, excluding `request.exclude`, as candidates ranked by the
/// reviewer weights downstream. Without a graph distance they are gated as distance 3.
pub fn to_candidates(hits: Vec<SemanticHit>, request: &SemanticRequest) -> Vec<Candidate> {
    hits.into_iter()
        .filter(|h| h.cosine >= request.min_score && !request.exclude.contains(&h.node))
        .take(request.limit)
        .map(|h| {
            let mut c = Candidate::node(
                h.node,
                h.title,
                ItemKind::RelatedSymbol,
                DiffSide::Head,
                CandidateSource::Semantic,
            );
            c.relation = Some("semantic".to_owned());
            c.raw.semantic_similarity = semantic_signal(h.cosine);
            c.span = h.span;
            c
        })
        .collect()
}
