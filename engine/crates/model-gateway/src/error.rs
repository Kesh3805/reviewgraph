//! Error types of this crate (DOM-002 for the crate-level error, GW-001/GW-002 for
//! [`GatewayError`]).

use std::time::Duration;

use review_core::{Classify, CoreError, ErrorClass};

use crate::types::{ModelTier, PrivacyClass, ProviderId, SchemaErrorSummary};

/// Crate-level error (configuration and construction failures).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    /// The gateway was assembled with an invalid configuration.
    #[error("invalid gateway configuration: {0}")]
    Config(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::Config(_) => ErrorClass::InvalidInput,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransientKind {
    Timeout,
    Connect,
    Reset,
    Overloaded,
    ServerError(u16),
    TruncatedBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermanentKind {
    InvalidRequest,
    Auth,
    Forbidden,
    ModelNotFound,
    QuotaExhausted,
    ContextTooLarge,
    Refusal,
    ReplayMiss,
    UnsupportedParameter,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BudgetKind {
    Deadline,
    InputTokens,
    OutputTokens,
    Cost,
    Calls,
}

/// Which limiter dimension a rate limit refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateScope {
    /// Reported by the provider (HTTP 429).
    Provider,
    Requests,
    InputTokens,
    OutputTokens,
}

/// Why a model call failed. Retry policy and fallback eligibility live in `retry`/`classify`
/// (GW-002).
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("transient: {kind:?}")]
    Transient {
        kind: TransientKind,
        provider: Option<ProviderId>,
    },
    #[error("rate limited")]
    RateLimited {
        retry_after: Option<Duration>,
        provider: ProviderId,
        scope: RateScope,
    },
    #[error("permanent: {kind:?}: {detail}")]
    Permanent {
        kind: PermanentKind,
        provider: Option<ProviderId>,
        detail: String,
    },
    #[error("schema violation")]
    SchemaViolation {
        errors: Vec<SchemaErrorSummary>,
        repaired: bool,
    },
    #[error("budget exceeded: {kind:?}")]
    BudgetExceeded { kind: BudgetKind },
    #[error("no eligible provider for {tier:?}/{privacy:?}: {reason}")]
    NoEligibleProvider {
        tier: ModelTier,
        privacy: PrivacyClass,
        reason: String,
    },
    #[error("cancelled")]
    Cancelled,
}

impl GatewayError {
    /// Only `Transient` and `RateLimited` are retried by the retry loop.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient { .. } | Self::RateLimited { .. })
    }

    /// Whether the router may try the next candidate after this error (retries exhausted).
    pub fn fallback_eligible(&self) -> bool {
        match self {
            Self::Transient { .. } | Self::RateLimited { .. } => true,
            Self::Permanent { kind, .. } => matches!(
                kind,
                PermanentKind::Auth
                    | PermanentKind::Forbidden
                    | PermanentKind::ModelNotFound
                    | PermanentKind::QuotaExhausted
                    | PermanentKind::UnsupportedParameter
            ),
            Self::SchemaViolation { .. }
            | Self::BudgetExceeded { .. }
            | Self::NoEligibleProvider { .. }
            | Self::Cancelled => false,
        }
    }

    /// Stable class string, used as a metric label and in `reviewer_runs.error_class`.
    pub fn class(&self) -> &'static str {
        match self {
            Self::Transient { .. } => "transient",
            Self::RateLimited { .. } => "rate_limited",
            Self::Permanent { .. } => "permanent",
            Self::SchemaViolation { .. } => "schema_violation",
            Self::BudgetExceeded { .. } => "budget_exceeded",
            Self::NoEligibleProvider { .. } => "no_eligible_provider",
            Self::Cancelled => "cancelled",
        }
    }

    /// The pipeline-wide error class this failure maps to.
    pub fn error_class(&self) -> ErrorClass {
        match self {
            Self::Transient { .. } => ErrorClass::Transient,
            Self::RateLimited { .. } => ErrorClass::RateLimited,
            Self::Cancelled => ErrorClass::Cancelled,
            Self::Permanent { .. }
            | Self::SchemaViolation { .. }
            | Self::BudgetExceeded { .. }
            | Self::NoEligibleProvider { .. } => ErrorClass::Permanent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_library_error_wraps_core_error() {
        let e = Error::from(CoreError::OutOfRange {
            field: "x",
            value: "1".into(),
        });
        assert_eq!(e.class(), ErrorClass::InvalidInput);
    }

    #[test]
    fn error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<Error>();
        assert_bounds::<GatewayError>();
    }
}
