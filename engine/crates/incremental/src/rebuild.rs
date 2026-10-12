//! Full-rebuild trigger evaluation (INC-011, PRD section 24).
//!
//! [`evaluate`] is a pure function of the base snapshot's recorded versions, the versions the
//! current binaries would produce, the change summary and operator flags. It never performs I/O:
//! the caller recomputes config-component hashes for changed config paths and passes them in.

use std::collections::BTreeMap;

/// Health of the base snapshot as recorded in its metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseStatus {
    Ready,
    NotReady,
    Inconsistent,
}

/// Versions recorded on a snapshot or produced by the current binaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionSet {
    pub graph_schema: u32,
    pub linker: String,
    /// Analyzer version per language name.
    pub analyzers: BTreeMap<String, String>,
    /// Hash per config component name (tsconfig, workspaces, source roots, ...).
    pub config: BTreeMap<String, String>,
}

/// Operator and pipeline flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RebuildFlags {
    pub explicit: bool,
    pub consistency_failed: bool,
    pub inbound_escalation: bool,
}

/// Inputs to [`evaluate`].
#[derive(Debug, Clone)]
pub struct RebuildInputs<'a> {
    pub base_status: BaseStatus,
    pub base: &'a VersionSet,
    pub head: &'a VersionSet,
    /// Languages present in the repository.
    pub languages: &'a [String],
    pub changed_files: u64,
    pub base_file_count: u64,
    /// Maximum fraction of changed files before a rebuild is cheaper.
    pub max_change_ratio: f32,
    pub chain_depth: u32,
    pub max_chain_depth: u32,
    pub delta_edge_churn: f32,
    pub max_edge_churn: f32,
    pub flags: RebuildFlags,
}

/// Why a full rebuild was chosen, ordered by severity.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum RebuildReason {
    Explicit,
    BaseNotReady,
    BaseInconsistent,
    ConsistencyCheckFailed,
    GraphSchemaChanged { from: u32, to: u32 },
    AnalyzerVersionChanged { language: String },
    LinkerVersionChanged,
    ConfigChanged { component: String },
    ChangeRatioTooHigh { ratio: f32 },
    InboundEscalation,
}

/// How a full rebuild proceeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullMode {
    Reparse,
    RelinkFromCache,
}

/// The decision.
#[derive(Debug, Clone, PartialEq)]
pub enum RebuildDecision {
    Incremental,
    CompactThenIncremental,
    Full {
        reasons: Vec<RebuildReason>,
        mode: FullMode,
    },
}

/// Evaluates every rule, collects reasons in rule order, and picks the mode.
#[must_use]
pub fn evaluate(inp: &RebuildInputs<'_>) -> RebuildDecision {
    let mut reasons = Vec::new();
    if inp.flags.explicit {
        reasons.push(RebuildReason::Explicit);
    }
    match inp.base_status {
        BaseStatus::Ready => {}
        BaseStatus::NotReady => reasons.push(RebuildReason::BaseNotReady),
        BaseStatus::Inconsistent => reasons.push(RebuildReason::BaseInconsistent),
    }
    if inp.flags.consistency_failed {
        reasons.push(RebuildReason::ConsistencyCheckFailed);
    }
    if inp.base.graph_schema != inp.head.graph_schema {
        reasons.push(RebuildReason::GraphSchemaChanged {
            from: inp.base.graph_schema,
            to: inp.head.graph_schema,
        });
    }
    for language in inp.languages {
        if inp.base.analyzers.get(language) != inp.head.analyzers.get(language) {
            reasons.push(RebuildReason::AnalyzerVersionChanged {
                language: language.clone(),
            });
        }
    }
    let linker_changed = inp.base.linker != inp.head.linker;
    if linker_changed {
        reasons.push(RebuildReason::LinkerVersionChanged);
    }
    let components: std::collections::BTreeSet<&String> = inp
        .base
        .config
        .keys()
        .chain(inp.head.config.keys())
        .collect();
    for component in components {
        if inp.base.config.get(component) != inp.head.config.get(component) {
            reasons.push(RebuildReason::ConfigChanged {
                component: component.clone(),
            });
        }
    }
    if inp.base_file_count > 0 {
        #[allow(clippy::cast_precision_loss)]
        let ratio = inp.changed_files as f32 / inp.base_file_count as f32;
        if ratio > inp.max_change_ratio {
            reasons.push(RebuildReason::ChangeRatioTooHigh { ratio });
        }
    }
    if inp.flags.inbound_escalation {
        reasons.push(RebuildReason::InboundEscalation);
    }

    if reasons.is_empty() {
        if inp.chain_depth >= inp.max_chain_depth || inp.delta_edge_churn >= inp.max_edge_churn {
            return RebuildDecision::CompactThenIncremental;
        }
        return RebuildDecision::Incremental;
    }
    // A linker bump alone can re-link from cached IR; anything else needs a re-parse.
    let relink_only = reasons
        .iter()
        .all(|r| matches!(r, RebuildReason::LinkerVersionChanged));
    reasons.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    RebuildDecision::Full {
        reasons,
        mode: if relink_only {
            FullMode::RelinkFromCache
        } else {
            FullMode::Reparse
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions() -> VersionSet {
        VersionSet {
            graph_schema: 1,
            linker: "l1".into(),
            analyzers: BTreeMap::from([("typescript".to_owned(), "1.0.0".to_owned())]),
            config: BTreeMap::from([("tsconfig".to_owned(), "h1".to_owned())]),
        }
    }

    fn run(
        mutate: impl FnOnce(&mut VersionSet, &mut RebuildFlags, &mut u64, &mut u32),
    ) -> RebuildDecision {
        let base = versions();
        let mut head = versions();
        let mut flags = RebuildFlags::default();
        let mut changed = 5;
        let mut depth = 1;
        mutate(&mut head, &mut flags, &mut changed, &mut depth);
        let languages = vec!["typescript".to_owned()];
        evaluate(&RebuildInputs {
            base_status: BaseStatus::Ready,
            base: &base,
            head: &head,
            languages: &languages,
            changed_files: changed,
            base_file_count: 100,
            max_change_ratio: 0.5,
            chain_depth: depth,
            max_chain_depth: 20,
            delta_edge_churn: 0.0,
            max_edge_churn: 0.1,
            flags,
        })
    }

    #[test]
    fn same_versions_and_small_change_is_incremental() {
        assert_eq!(run(|_, _, _, _| {}), RebuildDecision::Incremental);
    }

    #[test]
    fn schema_bump_forces_full_reparse() {
        let d = run(|h, _, _, _| h.graph_schema = 2);
        assert!(matches!(
            d,
            RebuildDecision::Full {
                mode: FullMode::Reparse,
                ..
            }
        ));
    }

    #[test]
    fn analyzer_minor_bump_forces_full_reparse() {
        let d = run(|h, _, _, _| {
            h.analyzers.insert("typescript".into(), "1.1.0".into());
        });
        assert!(matches!(
            d,
            RebuildDecision::Full {
                mode: FullMode::Reparse,
                ..
            }
        ));
    }

    #[test]
    fn linker_bump_relinks_from_cache() {
        let d = run(|h, _, _, _| h.linker = "l2".into());
        assert!(matches!(
            d,
            RebuildDecision::Full {
                mode: FullMode::RelinkFromCache,
                ..
            }
        ));
    }

    #[test]
    fn config_change_forces_full() {
        let d = run(|h, _, _, _| {
            h.config.insert("tsconfig".into(), "h2".into());
        });
        assert!(matches!(
            d,
            RebuildDecision::Full {
                mode: FullMode::Reparse,
                ..
            }
        ));
    }

    #[test]
    fn explicit_flag_wins() {
        let d = run(|_, f, _, _| f.explicit = true);
        match d {
            RebuildDecision::Full { reasons, .. } => {
                assert_eq!(reasons[0], RebuildReason::Explicit)
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn half_the_repo_changed_forces_full() {
        let d = run(|_, _, c, _| *c = 60);
        assert!(matches!(d, RebuildDecision::Full { .. }));
    }

    #[test]
    fn deep_chain_requests_compaction_not_rebuild() {
        assert_eq!(
            run(|_, _, _, d| *d = 20),
            RebuildDecision::CompactThenIncremental
        );
    }

    #[test]
    fn reasons_sorted_deterministically() {
        let d = run(|h, f, _, _| {
            h.linker = "l2".into();
            h.graph_schema = 2;
            f.explicit = true;
        });
        match d {
            RebuildDecision::Full { reasons, .. } => {
                assert_eq!(reasons[0], RebuildReason::Explicit);
                assert_eq!(
                    *reasons.last().unwrap(),
                    RebuildReason::LinkerVersionChanged
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
