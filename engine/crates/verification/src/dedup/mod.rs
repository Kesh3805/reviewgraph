//! Deduplication (DED-001..DED-004). Spec: docs/reviewers/deduplication.md.

pub mod fingerprint;
pub mod identity;
pub mod merge;
pub mod normalize;

pub use fingerprint::{
    root_cause_fingerprint, DedupError, FingerprintBasis, Predicate, RootCauseFingerprint,
    RootCauseInput,
};
pub use normalize::{normalize_claim, NormalizedClaim};
