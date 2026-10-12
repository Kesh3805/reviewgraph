//! Inverted identifier index (CTX-004) and the per-snapshot LRU of built indexes.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use review_core::ids::SymbolKey;

use super::tokenize::{skip_literal, tokenize};

/// Postings of tokens appearing in more nodes than this are ignored for candidate generation.
pub const DF_CUTOFF: usize = 500;
/// Index build refuses to grow beyond this many postings (OOM guard).
pub const MAX_POSTINGS: usize = 5_000_000;
/// Full identifiers weigh 1.0 in a query, sub-tokens this much.
pub const SUBTOKEN_WEIGHT: f32 = 0.3;
pub const MIN_SCORE: f32 = 0.25;
pub const MAX_HITS: usize = 50;
/// Built indexes kept in memory.
pub const CACHE_CAPACITY: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LexicalError {
    #[error("lexical index would exceed {MAX_POSTINGS} postings")]
    TooManyPostings,
}

/// The identifiers of one graph node: simple name, qualified-name segments, literal
/// identifiers from syntax facts (DI tokens, event names, config keys) and unresolved
/// reference names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexicalDoc {
    pub node: SymbolKey,
    pub identifiers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LexicalHit {
    pub node: SymbolKey,
    /// `sum(w * idf over matched tokens) / sum(w * idf over query tokens)`, in `[0, 1]`.
    pub score: f32,
}

#[derive(Debug, Default)]
pub struct LexicalIndex {
    postings: HashMap<String, Vec<SymbolKey>>,
    nodes: usize,
    total_postings: usize,
}

impl LexicalIndex {
    /// Builds the index. Postings are sorted by node key, so the result is deterministic.
    pub fn build(docs: impl IntoIterator<Item = LexicalDoc>) -> Result<Self, LexicalError> {
        let mut postings: HashMap<String, Vec<SymbolKey>> = HashMap::new();
        let mut nodes = 0usize;
        let mut total = 0usize;
        for doc in docs {
            nodes += 1;
            let mut seen: Vec<String> = Vec::new();
            for ident in doc.identifiers.iter().filter(|i| !skip_literal(i)) {
                let t = tokenize(ident);
                if t.full.is_empty() {
                    continue;
                }
                for tok in std::iter::once(t.full).chain(t.subs) {
                    if !seen.contains(&tok) {
                        seen.push(tok);
                    }
                }
            }
            for tok in seen {
                total += 1;
                if total > MAX_POSTINGS {
                    return Err(LexicalError::TooManyPostings);
                }
                postings.entry(tok).or_default().push(doc.node);
            }
        }
        for list in postings.values_mut() {
            list.sort();
            list.dedup();
        }
        Ok(Self {
            postings,
            nodes,
            total_postings: total,
        })
    }

    pub fn node_count(&self) -> usize {
        self.nodes
    }

    pub fn postings_count(&self) -> usize {
        self.total_postings
    }

    pub fn df(&self, token: &str) -> usize {
        self.postings.get(token).map_or(0, Vec::len)
    }

    /// `ln(1 + N / df)`; an absent token counts as `df = 1`.
    pub fn idf(&self, token: &str) -> f32 {
        let df = self.df(token).max(1);
        (1.0 + self.nodes as f32 / df as f32).ln()
    }

    /// Scores nodes against changed identifiers. Query tokens: full identifier weight 1.0,
    /// sub-tokens 0.3 (the larger weight wins). Tokens over [`DF_CUTOFF`] are ignored. Returns
    /// at most [`MAX_HITS`] hits with score >= [`MIN_SCORE`], best first (ties by node key).
    pub fn query(&self, identifiers: &[String]) -> Vec<LexicalHit> {
        let mut weights: BTreeMap<String, f32> = BTreeMap::new();
        for ident in identifiers.iter().filter(|i| !skip_literal(i)) {
            let t = tokenize(ident);
            if t.full.is_empty() {
                continue;
            }
            let slot = weights.entry(t.full).or_insert(0.0);
            *slot = slot.max(1.0);
            for sub in t.subs {
                let slot = weights.entry(sub).or_insert(0.0);
                *slot = slot.max(SUBTOKEN_WEIGHT);
            }
        }
        weights.retain(|tok, _| self.df(tok) <= DF_CUTOFF);
        let denom: f32 = weights.iter().map(|(t, w)| w * self.idf(t)).sum();
        if denom <= 0.0 {
            return Vec::new();
        }
        let mut sums: BTreeMap<SymbolKey, f32> = BTreeMap::new();
        for (tok, w) in &weights {
            let Some(list) = self.postings.get(tok) else {
                continue;
            };
            let contribution = w * self.idf(tok);
            for node in list {
                *sums.entry(*node).or_insert(0.0) += contribution;
            }
        }
        let mut hits: Vec<LexicalHit> = sums
            .into_iter()
            .map(|(node, sum)| LexicalHit {
                node,
                score: (sum / denom).clamp(0.0, 1.0),
            })
            .filter(|h| h.score >= MIN_SCORE)
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.node.cmp(&b.node))
        });
        hits.truncate(MAX_HITS);
        hits
    }
}

/// LRU of built indexes keyed by snapshot id.
#[derive(Debug)]
pub struct LexicalIndexCache {
    capacity: usize,
    entries: Mutex<Vec<(String, Arc<LexicalIndex>)>>,
}

impl Default for LexicalIndexCache {
    fn default() -> Self {
        Self::new(CACHE_CAPACITY)
    }
}

impl LexicalIndexCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Mutex::new(Vec::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the cached index for `snapshot`, or builds it once. A build failure is returned
    /// and nothing is cached.
    pub fn get_or_build(
        &self,
        snapshot: &str,
        build: impl FnOnce() -> Result<LexicalIndex, LexicalError>,
    ) -> Result<Arc<LexicalIndex>, LexicalError> {
        let Ok(mut entries) = self.entries.lock() else {
            return build().map(Arc::new);
        };
        if let Some(pos) = entries.iter().position(|(k, _)| k == snapshot) {
            let entry = entries.remove(pos);
            let index = Arc::clone(&entry.1);
            entries.push(entry);
            return Ok(index);
        }
        let index = Arc::new(build()?);
        if entries.len() >= self.capacity {
            entries.remove(0);
        }
        entries.push((snapshot.to_owned(), Arc::clone(&index)));
        Ok(index)
    }
}
