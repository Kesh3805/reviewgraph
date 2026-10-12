//! Lexical candidates from changed identifiers (CTX-004).

use std::sync::Arc;

use review_core::location::DiffSide;

use super::{Candidate, CandidateGenerator, CandidatePool, GenStats, LineSpan, SelectionInput};
use crate::lexical::LexicalIndex;
use crate::model::{CandidateSource, ItemKind};

#[derive(Debug, Clone)]
pub struct LexicalCandidates {
    index: Arc<LexicalIndex>,
    /// Names in added/removed calls, literal identifiers in added/removed facts and the names
    /// of changed symbols (CHG-003).
    changed_identifiers: Vec<String>,
}

impl LexicalCandidates {
    pub fn new(index: Arc<LexicalIndex>, changed_identifiers: Vec<String>) -> Self {
        Self {
            index,
            changed_identifiers,
        }
    }
}

impl CandidateGenerator for LexicalCandidates {
    fn source(&self) -> CandidateSource {
        CandidateSource::Lexical
    }

    fn generate(&self, input: &SelectionInput<'_>, out: &mut CandidatePool) -> GenStats {
        let mut stats = GenStats::default();
        let seeds = input.seed_keys();
        for hit in self.index.query(&self.changed_identifiers) {
            if seeds.contains(&hit.node) {
                continue;
            }
            if let Some(existing) = out.get_node_mut(hit.node, DiffSide::Head) {
                // Already a candidate (structural or otherwise): add the source and the signal.
                existing.sources.insert(CandidateSource::Lexical);
                existing.raw.changed_code_similarity =
                    existing.raw.changed_code_similarity.max(hit.score);
                stats.merged += 1;
                continue;
            }
            let info = input
                .locator
                .and_then(|l| l.locate(hit.node, DiffSide::Head));
            let title = info
                .as_ref()
                .map_or_else(|| hit.node.to_string(), |i| i.title.clone());
            let mut c = Candidate::node(
                hit.node,
                title,
                ItemKind::RelatedSymbol,
                DiffSide::Head,
                CandidateSource::Lexical,
            );
            c.relation = Some("lexical".to_owned());
            c.raw.changed_code_similarity = hit.score;
            c.span = info.map(|i| LineSpan {
                file: i.file,
                start_line: i.start_line,
                end_line: i.end_line,
                side: DiffSide::Head,
            });
            stats.record(out.insert(c));
        }
        stats
    }
}
