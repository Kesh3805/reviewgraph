//! Test candidates (CTX-003): changed tests and tests mapped by the impact graph.

use impact::Relation;
use review_core::location::DiffSide;

use super::{
    convert_path, Candidate, CandidateGenerator, CandidatePool, GenStats, LineSpan, SelectionInput,
};
use crate::model::{CandidateSource, ItemKind};

/// Impact tests below this mapping score are not candidates.
pub const MIN_TEST_SCORE: f32 = 0.6;
/// Tests that only mock the seed's class are down-weighted by this factor.
pub const MOCKED_FACTOR: f32 = 0.5;

#[derive(Debug, Clone, Copy, Default)]
pub struct TestCandidates;

impl CandidateGenerator for TestCandidates {
    fn source(&self) -> CandidateSource {
        CandidateSource::TestMap
    }

    fn generate(&self, input: &SelectionInput<'_>, out: &mut CandidatePool) -> GenStats {
        let mut stats = GenStats::default();
        for seed in input.seeds.iter().filter(|s| s.is_test && !s.skipped) {
            let mut c = Candidate::node(
                seed.key,
                seed.title.clone(),
                ItemKind::Test,
                seed.side,
                CandidateSource::TestMap,
            );
            c.distance = Some(0);
            c.pinned = true;
            c.raw.test_relationship = 1.0;
            c.raw.risk_importance = seed.risk.clamp(0.0, 1.0);
            c.span = Some(LineSpan {
                file: seed.file.clone(),
                start_line: seed.start_line,
                end_line: seed.end_line,
                side: seed.side,
            });
            stats.record(out.insert(c));
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
            for el in impact.of(Relation::Test) {
                let Some(mapping) = el.test.as_ref() else {
                    stats.skipped += 1;
                    continue;
                };
                if mapping.score < MIN_TEST_SCORE {
                    stats.skipped += 1;
                    continue;
                }
                let info = input
                    .locator
                    .and_then(|l| l.locate(el.node, DiffSide::Head));
                let title = info
                    .as_ref()
                    .map_or_else(|| el.node_id.clone(), |i| i.title.clone());
                let mut c = Candidate::node(
                    el.node,
                    title,
                    ItemKind::Test,
                    DiffSide::Head,
                    CandidateSource::TestMap,
                );
                c.distance = Some(el.distance);
                c.path = convert_path(&el.path);
                c.min_confidence = el.min_confidence.as_f32();
                c.relation = Some(Relation::Test.as_str().to_owned());
                c.weak = el.weak;
                c.raw.test_relationship = if mapping.mocked {
                    mapping.score * MOCKED_FACTOR
                } else {
                    mapping.score
                };
                c.span = info.map(|i| LineSpan {
                    file: i.file,
                    start_line: i.start_line,
                    end_line: i.end_line,
                    side: DiffSide::Head,
                });
                stats.record(out.insert(c));
            }
        }
        stats
    }
}
