//! Convention miner framework (PROF-003): samples, violations, consistency, exceptions,
//! confidence and scope. Concrete miners (PROF-004/005) register in [`registry`].

pub mod confidence;
pub mod miner;
pub mod registry;
pub mod scope;

pub use confidence::is_enforceable;
pub use miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
pub use registry::{mine_one, MineOptions, MinerRegistry};

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use review_core::ids::{RepositoryId, SnapshotId};
    use review_core::version::ConfigHash;

    use super::*;
    use crate::model::RepositoryProfile;

    struct NoGraph;
    impl GraphView for NoGraph {}

    /// `n` samples spread over `modules`; every one conforms except those whose index is in `bad`.
    struct Fixed {
        n: usize,
        modules: usize,
        bad: Vec<usize>,
        annotate_bad: bool,
    }

    impl ConventionMiner for Fixed {
        fn id(&self) -> &'static str {
            "fixed"
        }
        fn version(&self) -> u32 {
            1
        }
        fn rule(&self) -> &'static str {
            "fixed rule"
        }
        fn population(&self, _: &dyn GraphView, _: &PartialProfile) -> Vec<Sample> {
            (0..self.n)
                .map(|i| {
                    let module = format!("src/modules/m{}", i % self.modules);
                    Sample {
                        id: format!("s{i}"),
                        path: format!("{module}/f{i}.ts"),
                        module,
                        ignore_annotations: if self.annotate_bad && self.bad.contains(&i) {
                            vec!["fixed".into()]
                        } else {
                            vec![]
                        },
                    }
                })
                .collect()
        }
        fn conforms(&self, s: &Sample, _: &dyn GraphView) -> Conformance {
            let i: usize = s.id[1..].parse().unwrap();
            if self.bad.contains(&i) {
                Conformance::Violates {
                    evidence: "bad".into(),
                }
            } else {
                Conformance::Conforms
            }
        }
    }

    fn partial() -> RepositoryProfile {
        RepositoryProfile::empty(
            RepositoryId::new(),
            SnapshotId::new(),
            ConfigHash::of(b"c"),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        )
    }

    fn run(m: &Fixed) -> Option<crate::model::Convention> {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        mine_one(m, &NoGraph, &partial(), &MineOptions::default(), now)
    }

    #[test]
    fn single_module_pattern_low_confidence() {
        let c = run(&Fixed {
            n: 38,
            modules: 1,
            bad: vec![0],
            annotate_bad: false,
        })
        .unwrap();
        assert!(c.confidence < 0.9);
        let c = run(&Fixed {
            n: 38,
            modules: 3,
            bad: vec![0],
            annotate_bad: false,
        })
        .unwrap();
        assert!(c.confidence >= 0.9);
        assert_eq!((c.samples, c.violations), (38, 1));
    }

    #[test]
    fn review_ignore_annotation_is_exception() {
        let c = run(&Fixed {
            n: 20,
            modules: 3,
            bad: vec![0],
            annotate_bad: true,
        })
        .unwrap();
        assert_eq!(c.violations, 0);
        assert_eq!(c.exceptions.len(), 1);
    }

    #[test]
    fn below_threshold_is_informational() {
        let c = run(&Fixed {
            n: 6,
            modules: 3,
            bad: vec![],
            annotate_bad: false,
        })
        .unwrap();
        assert!(!is_enforceable(c.confidence, c.samples));
    }

    #[test]
    fn scope_narrowing_picks_broadest_consistent() {
        let c = run(&Fixed {
            n: 30,
            modules: 3,
            bad: vec![],
            annotate_bad: false,
        })
        .unwrap();
        assert_eq!(c.scope, "**");
        // Violations concentrated in module m0 make the root inconsistent (< 0.9 overall), and
        // src/modules/** too, so the first consistent module-level scope wins.
        let bad: Vec<usize> = (0..30).filter(|i| i % 3 == 0).collect();
        let c = run(&Fixed {
            n: 30,
            modules: 3,
            bad,
            annotate_bad: false,
        })
        .unwrap();
        assert_eq!(c.scope, "src/modules/m1/**");
    }

    #[test]
    fn empty_population_yields_no_convention() {
        assert!(run(&Fixed {
            n: 0,
            modules: 1,
            bad: vec![],
            annotate_bad: false
        })
        .is_none());
    }

    #[test]
    fn registry_is_deterministic() {
        let mut r = MinerRegistry::new();
        r.register(Box::new(Fixed {
            n: 12,
            modules: 3,
            bad: vec![],
            annotate_bad: false,
        }));
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let a = r.mine_all(&NoGraph, &partial(), &MineOptions::default(), now);
        let b = r.mine_all(&NoGraph, &partial(), &MineOptions::default(), now);
        assert_eq!(a, b);
        assert_eq!(r.versions(), vec![("fixed", 1)]);
    }
}
