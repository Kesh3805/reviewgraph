//! Per-module regression signals (HIST-003).
//!
//! A regression is a bugfix (or revert) pull request that touches a module's symbols within
//! [`REGRESSION_WINDOW_DAYS`] after another change to the same symbols. Symbols are matched by
//! lineage id, which stays stable across renames, never by name.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};

use super::signals::{derive_signal, Signal, SignalKind, SignalObservation, SignalScope};

pub const REGRESSION_WINDOW_DAYS: i64 = 90;
/// The window the statistic is reported over.
const STATISTIC_WINDOW_DAYS: u32 = 365;

/// The symbols of one module a pull request changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleChange {
    pub module: String,
    /// Symbol lineage ids (stable across renames).
    pub lineage_ids: Vec<String>,
}

/// A merged pull request in history, classified at ingest time (titles are stored hashed, so the
/// bugfix heuristic must run before hashing; see [`looks_like_bugfix`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRecord {
    pub pr_id: String,
    pub author_hash: String,
    pub closed_at: DateTime<Utc>,
    pub is_bugfix: bool,
    pub is_revert: bool,
    pub modules: Vec<ModuleChange>,
}

/// Title and label heuristic for a bugfix pull request.
pub fn looks_like_bugfix(title: &str, labels: &[String]) -> bool {
    let title = title.to_lowercase();
    let by_title = title.starts_with("fix")
        || title.starts_with("bugfix")
        || title.starts_with("hotfix")
        || title.contains("fixes #")
        || title.contains("regression");
    let by_label = labels.iter().any(|l| {
        let l = l.to_lowercase();
        l == "bug" || l == "bugfix" || l == "regression" || l == "hotfix"
    });
    by_title || by_label
}

/// Per-module observations: every change to a module is one observation, positive when it is a
/// regression of an earlier change.
pub fn module_observations(history: &[ChangeRecord]) -> BTreeMap<String, Vec<SignalObservation>> {
    let mut ordered: Vec<&ChangeRecord> = history.iter().collect();
    ordered.sort_by(|a, b| (a.closed_at, &a.pr_id).cmp(&(b.closed_at, &b.pr_id)));
    let window = Duration::days(REGRESSION_WINDOW_DAYS);
    let mut out: BTreeMap<String, Vec<SignalObservation>> = BTreeMap::new();
    for (i, record) in ordered.iter().enumerate() {
        for change in &record.modules {
            let mine: BTreeSet<&str> = change.lineage_ids.iter().map(String::as_str).collect();
            let regression = (record.is_bugfix || record.is_revert)
                && ordered[..i].iter().any(|earlier| {
                    earlier.pr_id != record.pr_id
                        && record.closed_at - earlier.closed_at <= window
                        && earlier.modules.iter().any(|m| {
                            m.module == change.module
                                && m.lineage_ids.iter().any(|id| mine.contains(id.as_str()))
                        })
                });
            out.entry(change.module.clone())
                .or_default()
                .push(SignalObservation {
                    pr_id: record.pr_id.clone(),
                    author_hash: record.author_hash.clone(),
                    at: record.closed_at,
                    positive: regression,
                });
        }
    }
    out
}

/// Regression signals for every module with enough evidence.
pub fn module_regression_signals(history: &[ChangeRecord], now: DateTime<Utc>) -> Vec<Signal> {
    module_observations(history)
        .into_iter()
        .filter_map(|(module, observations)| {
            derive_signal(
                SignalKind::ModuleRegression,
                SignalScope::Module(module),
                &observations,
                STATISTIC_WINDOW_DAYS,
                now,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0)
            .single()
            .unwrap_or_default()
    }

    fn record(
        id: &str,
        days_ago: i64,
        bugfix: bool,
        module: &str,
        lineage: &[&str],
    ) -> ChangeRecord {
        ChangeRecord {
            pr_id: id.to_owned(),
            author_hash: format!("author-{}", id.len() % 3),
            closed_at: now() - Duration::days(days_ago),
            is_bugfix: bugfix,
            is_revert: false,
            modules: vec![ModuleChange {
                module: module.to_owned(),
                lineage_ids: lineage.iter().map(|s| (*s).to_owned()).collect(),
            }],
        }
    }

    /// `payments` has 6 changes, 3 of them bugfixes shortly after changes to the same symbols;
    /// `docs` has only 4 changes.
    fn fixture() -> Vec<ChangeRecord> {
        vec![
            record("p1", 100, false, "payments", &["L1"]),
            record("p2", 95, true, "payments", &["L1"]),
            record("p3", 80, false, "payments", &["L2"]),
            record("p4", 60, true, "payments", &["L2"]),
            record("p5", 40, false, "payments", &["L3"]),
            record("p6", 20, true, "payments", &["L3"]),
            record("d1", 50, false, "docs", &["D1"]),
            record("d2", 40, false, "docs", &["D2"]),
            record("d3", 30, true, "docs", &["D2"]),
            record("d4", 20, false, "docs", &["D3"]),
        ]
    }

    #[test]
    fn regression_prone_module_gets_signal_and_thin_module_none() {
        let signals = module_regression_signals(&fixture(), now());
        assert_eq!(signals.len(), 1, "docs has n = 4 < 5");
        let s = &signals[0];
        assert_eq!(s.scope, SignalScope::Module("payments".into()));
        assert_eq!((s.n, s.statistic.count), (6, 3));
        assert!(
            s.statistic.rate > 0.3 && s.statistic.rate < 0.7,
            "{}",
            s.statistic.rate
        );
        assert_eq!(s.evidence_refs, vec!["p2", "p4", "p6"]);
    }

    #[test]
    fn regression_detection_follows_symbol_lineage_across_rename() {
        // The symbol is renamed between the change and the fix; the lineage id is the same.
        let history = vec![
            record("a", 30, false, "m", &["lineage-1"]),
            record("b", 10, true, "m", &["lineage-1"]),
        ];
        let obs = module_observations(&history);
        assert_eq!(
            obs["m"].iter().map(|o| o.positive).collect::<Vec<_>>(),
            [false, true]
        );
        // A different lineage is not a regression of it.
        let unrelated = vec![
            record("a", 30, false, "m", &["lineage-1"]),
            record("b", 10, true, "m", &["lineage-2"]),
        ];
        assert!(!module_observations(&unrelated)["m"][1].positive);
    }

    #[test]
    fn fix_outside_the_window_is_not_a_regression() {
        let history = vec![
            record("a", 400, false, "m", &["L"]),
            record("b", 10, true, "m", &["L"]),
        ];
        assert!(!module_observations(&history)["m"][1].positive);
    }

    #[test]
    fn bugfix_heuristic() {
        assert!(looks_like_bugfix("Fix null pointer in checkout", &[]));
        assert!(looks_like_bugfix("Update retries", &["Bug".to_owned()]));
        assert!(!looks_like_bugfix(
            "Add pricing page",
            &["feature".to_owned()]
        ));
    }
}
