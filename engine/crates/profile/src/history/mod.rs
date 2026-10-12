//! History-derived signals and their admission gate (HIST-003, HIST-004; PRD §67–§69).
//!
//! Signals are advisory aggregates ("signals, not rules"). Nothing here can create a finding or
//! suppress one on its own: the [`admission`] gate decides what a signal may influence, and
//! `history.enabled = false` switches every influence off at once.

pub mod admission;
pub mod modules;
pub mod signals;

pub use admission::{
    admit, admit_or_reject, Acceptance, AdmissionDecision, AdmissionInput, AdmissionOutcome,
    Corroboration, Observation, Origin, Reason, PATTERN_SUPPRESSION_TTL_DAYS,
    RANKING_ONLY_MAX_WEIGHT,
};
pub use modules::{
    looks_like_bugfix, module_regression_signals, ChangeRecord, ModuleChange,
    REGRESSION_WINDOW_DAYS,
};
pub use signals::{
    context_bonus, derive_signal, risk_bump, HistoryInfluence, Signal, SignalKind,
    SignalObservation, SignalScope, SignalSet, Statistic, DECAY_HALF_LIFE_DAYS, MAX_CONTEXT_WEIGHT,
    MAX_RISK_BUMP, MIN_DISTINCT_PRS, MIN_OBSERVATIONS,
};
