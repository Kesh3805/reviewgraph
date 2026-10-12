//! Signal records, derivation with anti-noise minimums, decay, and the bounded uses (HIST-003).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use review_core::ids::SymbolKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Minimum observations before a signal is emitted.
pub const MIN_OBSERVATIONS: u32 = 5;
/// Minimum distinct pull requests behind a signal.
pub const MIN_DISTINCT_PRS: u32 = 3;
/// Evidence loses half its weight every 180 days.
pub const DECAY_HALF_LIFE_DAYS: f64 = 180.0;
/// The risk engine may add at most this much to module risk.
pub const MAX_RISK_BUMP: f64 = 0.10;
/// Context ranking may add at most this much score.
pub const MAX_CONTEXT_WEIGHT: f64 = 0.15;
/// Signals are recomputed nightly; two days of validity leaves room for a missed run.
pub const SIGNAL_VALIDITY_DAYS: i64 = 2;
pub const SIGNAL_VERSION: u32 = 1;
const MAX_EVIDENCE_REFS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    ModuleRegression,
    FindingDensity,
    AcceptedPattern,
    DismissalPattern,
    ChangeClassNeedsTests,
}

impl SignalKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "module_regression" => Some(Self::ModuleRegression),
            "finding_density" => Some(Self::FindingDensity),
            "accepted_pattern" => Some(Self::AcceptedPattern),
            "dismissal_pattern" => Some(Self::DismissalPattern),
            "change_class_needs_tests" => Some(Self::ChangeClassNeedsTests),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModuleRegression => "module_regression",
            Self::FindingDensity => "finding_density",
            Self::AcceptedPattern => "accepted_pattern",
            Self::DismissalPattern => "dismissal_pattern",
            Self::ChangeClassNeedsTests => "change_class_needs_tests",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SignalScope {
    Module(String),
    /// Hex of the `SymbolKey`.
    Symbol(String),
    Path(String),
    ChangeClass(String),
}

impl SignalScope {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Stable id used by the admission audit trail.
    pub fn key(&self) -> String {
        match self {
            Self::Module(v) => format!("module:{v}"),
            Self::Symbol(v) => format!("symbol:{v}"),
            Self::Path(v) => format!("path:{v}"),
            Self::ChangeClass(v) => format!("change_class:{v}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Statistic {
    /// Decay-weighted share of positive observations, in `[0, 1]`.
    pub rate: f64,
    /// Raw number of positive observations.
    pub count: u32,
    pub window_days: u32,
}

/// An advisory signal: aggregates and ids only, never text or identities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub kind: SignalKind,
    pub scope: SignalScope,
    pub statistic: Statistic,
    /// Sample size.
    pub n: u32,
    /// Pull request ids that carry the positive observations.
    pub evidence_refs: Vec<String>,
    pub computed_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub version: u32,
}

impl Signal {
    /// Stable id of the signal across refreshes: `kind` and `scope`.
    pub fn signal_id(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.scope.key())
    }

    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        now < self.valid_until
    }

    pub fn evidence_json(&self) -> Value {
        json!(self.evidence_refs)
    }
}

/// One observation behind a signal: a pull request and whether it showed the pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalObservation {
    pub pr_id: String,
    pub author_hash: String,
    pub at: DateTime<Utc>,
    pub positive: bool,
}

/// `0.5^(age / half-life)`; evidence from the future counts fully.
pub fn decay_weight(age_days: f64) -> f64 {
    if age_days <= 0.0 {
        1.0
    } else {
        0.5_f64.powf(age_days / DECAY_HALF_LIFE_DAYS)
    }
}

/// Derives one signal, or `None` when the evidence is too thin: fewer than
/// [`MIN_OBSERVATIONS`] observations, fewer than [`MIN_DISTINCT_PRS`] distinct pull requests, or
/// no positive observation at all. A pure function of its inputs and `now`.
pub fn derive_signal(
    kind: SignalKind,
    scope: SignalScope,
    observations: &[SignalObservation],
    window_days: u32,
    now: DateTime<Utc>,
) -> Option<Signal> {
    let n = u32::try_from(observations.len()).unwrap_or(u32::MAX);
    let distinct_prs: BTreeSet<&str> = observations.iter().map(|o| o.pr_id.as_str()).collect();
    let distinct = u32::try_from(distinct_prs.len()).unwrap_or(u32::MAX);
    if n < MIN_OBSERVATIONS || distinct < MIN_DISTINCT_PRS {
        return None;
    }
    let (mut total, mut positive_weight, mut count) = (0.0_f64, 0.0_f64, 0_u32);
    let mut refs: BTreeSet<String> = BTreeSet::new();
    for o in observations {
        let age_days = (now - o.at).num_seconds() as f64 / 86_400.0;
        let w = decay_weight(age_days);
        total += w;
        if o.positive {
            positive_weight += w;
            count += 1;
            refs.insert(o.pr_id.clone());
        }
    }
    if count == 0 || total <= 0.0 {
        return None;
    }
    Some(Signal {
        kind,
        scope,
        statistic: Statistic {
            rate: (positive_weight / total).clamp(0.0, 1.0),
            count,
            window_days,
        },
        n,
        evidence_refs: refs.into_iter().take(MAX_EVIDENCE_REFS).collect(),
        computed_at: now,
        valid_until: now + Duration::days(SIGNAL_VALIDITY_DAYS),
        version: SIGNAL_VERSION,
    })
}

/// Switches from `history.signals.*`; both default to off, and `history.enabled = false`
/// overrides them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryInfluence {
    pub history_enabled: bool,
    pub risk: bool,
    pub context: bool,
}

impl HistoryInfluence {
    pub const fn risk_allowed(self) -> bool {
        self.history_enabled && self.risk
    }

    pub const fn context_allowed(self) -> bool {
        self.history_enabled && self.context
    }
}

/// A repository's current signals; replaced wholesale on refresh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SignalSet {
    signals: Vec<Signal>,
}

impl SignalSet {
    pub fn new(signals: Vec<Signal>) -> Self {
        Self { signals }
    }

    pub fn all(&self) -> &[Signal] {
        &self.signals
    }

    /// Valid signals whose scope covers any of `symbols`. `module_of` maps a symbol to its module.
    /// Stale or missing signals give an empty answer: a review proceeds with none.
    pub fn for_symbols<'a>(
        &'a self,
        symbols: &[SymbolKey],
        module_of: &BTreeMap<SymbolKey, String>,
        now: DateTime<Utc>,
    ) -> Vec<&'a Signal> {
        let keys: BTreeSet<String> = symbols.iter().map(ToString::to_string).collect();
        let modules: BTreeSet<&str> = symbols
            .iter()
            .filter_map(|s| module_of.get(s).map(String::as_str))
            .collect();
        self.signals
            .iter()
            .filter(|s| s.is_valid_at(now))
            .filter(|s| match &s.scope {
                SignalScope::Symbol(k) => keys.contains(k),
                SignalScope::Module(m) => modules.contains(m.as_str()),
                SignalScope::Path(_) | SignalScope::ChangeClass(_) => false,
            })
            .collect()
    }
}

fn strongest_rate(signals: &[&Signal], kind: SignalKind) -> f64 {
    signals
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| s.statistic.rate)
        .fold(0.0, f64::max)
}

/// The module risk after the advisory regression bump: at most [`MAX_RISK_BUMP`], never lower
/// than `base`, never above 1.0 and never above `ceiling` (the deterministic-risk override) unless
/// `base` already is.
pub fn risk_bump(
    base: f64,
    signals: &[&Signal],
    influence: HistoryInfluence,
    ceiling: Option<f64>,
) -> f64 {
    if !influence.risk_allowed() {
        return base;
    }
    let bump = (strongest_rate(signals, SignalKind::ModuleRegression) * MAX_RISK_BUMP)
        .clamp(0.0, MAX_RISK_BUMP);
    let mut risk = (base + bump).min(1.0);
    if let Some(ceiling) = ceiling {
        risk = risk.min(ceiling.max(base));
    }
    risk.max(base)
}

/// The extra context-ranking score for symbols in a regression-prone module: at most
/// [`MAX_CONTEXT_WEIGHT`], and exactly 0 unless context use is switched on.
pub fn context_bonus(signals: &[&Signal], influence: HistoryInfluence) -> f64 {
    if !influence.context_allowed() {
        return 0.0;
    }
    (strongest_rate(signals, SignalKind::ModuleRegression) * MAX_CONTEXT_WEIGHT)
        .clamp(0.0, MAX_CONTEXT_WEIGHT)
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

    fn obs(n: usize, prs: usize, positive_every: usize, age_days: i64) -> Vec<SignalObservation> {
        (0..n)
            .map(|i| SignalObservation {
                pr_id: format!("pr{}", i % prs),
                author_hash: format!("a{}", i % 3),
                at: now() - Duration::days(age_days),
                positive: i % positive_every == 0,
            })
            .collect()
    }

    fn scope() -> SignalScope {
        SignalScope::Module("payments".into())
    }

    #[test]
    fn signal_requires_min_sample_and_distinct_prs() {
        let derive = |o: &[SignalObservation]| {
            derive_signal(SignalKind::ModuleRegression, scope(), o, 365, now())
        };
        assert!(derive(&obs(4, 4, 1, 1)).is_none(), "n < 5");
        assert!(
            derive(&obs(8, 2, 1, 1)).is_none(),
            "fewer than 3 distinct PRs"
        );
        assert!(derive(&obs(5, 5, 1_000, 1)).is_some(), "i = 0 is positive");
        let stat = derive(&obs(5, 5, 1, 1)).map(|s| (s.n, s.statistic.rate));
        assert!(matches!(stat, Some((5, r)) if (r - 1.0).abs() < 1e-9));
    }

    #[test]
    fn no_positive_observation_means_no_signal() {
        let mut o = obs(6, 6, 1, 1);
        for x in &mut o {
            x.positive = false;
        }
        assert!(derive_signal(SignalKind::ModuleRegression, scope(), &o, 365, now()).is_none());
    }

    #[test]
    fn decay_reduces_old_evidence() {
        // Old positives followed by recent negatives: the old evidence weighs less than half.
        let mut o = Vec::new();
        for i in 0..3 {
            o.push(SignalObservation {
                pr_id: format!("old{i}"),
                author_hash: "a".into(),
                at: now() - Duration::days(360),
                positive: true,
            });
            o.push(SignalObservation {
                pr_id: format!("new{i}"),
                author_hash: "b".into(),
                at: now() - Duration::days(1),
                positive: false,
            });
        }
        let s = derive_signal(SignalKind::ModuleRegression, scope(), &o, 365, now());
        let rate = s.map_or(-1.0, |s| s.statistic.rate);
        assert!(rate > 0.0 && rate < 0.5 * 0.5 + 0.01, "rate {rate}");
        assert!((decay_weight(180.0) - 0.5).abs() < 1e-9);
    }

    fn sample_signal(rate: f64) -> Signal {
        Signal {
            kind: SignalKind::ModuleRegression,
            scope: scope(),
            statistic: Statistic {
                rate,
                count: 3,
                window_days: 365,
            },
            n: 8,
            evidence_refs: vec!["pr1".into()],
            computed_at: now(),
            valid_until: now() + Duration::days(2),
            version: 1,
        }
    }

    fn on() -> HistoryInfluence {
        HistoryInfluence {
            history_enabled: true,
            risk: true,
            context: true,
        }
    }

    #[test]
    fn risk_bump_is_bounded_and_never_lowers_risk() {
        let s = sample_signal(1.0);
        let refs = [&s];
        let bumped = risk_bump(0.40, &refs, on(), None);
        assert!((bumped - 0.50).abs() < 1e-9, "at most +0.10");
        assert!(risk_bump(0.95, &refs, on(), None) <= 1.0);
        // The deterministic override is a ceiling the bump cannot exceed...
        assert!((risk_bump(0.40, &refs, on(), Some(0.45)) - 0.45).abs() < 1e-9);
        // ...but a base above the ceiling is never lowered.
        assert!((risk_bump(0.60, &refs, on(), Some(0.45)) - 0.60).abs() < 1e-9);
        let weak = sample_signal(0.0);
        assert!((risk_bump(0.40, &[&weak], on(), None) - 0.40).abs() < 1e-9);
    }

    #[test]
    fn signals_default_off_do_not_change_scores() {
        let s = sample_signal(1.0);
        let off = HistoryInfluence::default();
        assert_eq!(context_bonus(&[&s], off), 0.0);
        assert_eq!(risk_bump(0.4, &[&s], off, None), 0.4);
        // Signal switches alone do not matter without the history kill switch.
        let killed = HistoryInfluence {
            history_enabled: false,
            risk: true,
            context: true,
        };
        assert_eq!(context_bonus(&[&s], killed), 0.0);
        assert!(context_bonus(&[&s], on()) <= MAX_CONTEXT_WEIGHT);
    }

    #[test]
    fn stale_signals_are_not_returned() {
        let set = SignalSet::new(vec![sample_signal(0.5)]);
        let key = SymbolKey::from_bytes([1; 16]);
        let modules = BTreeMap::from([(key, "payments".to_owned())]);
        assert_eq!(set.for_symbols(&[key], &modules, now()).len(), 1);
        assert!(set
            .for_symbols(&[key], &modules, now() + Duration::days(3))
            .is_empty());
        assert!(SignalSet::default()
            .for_symbols(&[key], &modules, now())
            .is_empty());
    }
}
