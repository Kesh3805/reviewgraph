//! `model-gateway` crate: the only way reviewers and verification reach a model (ADR-009).
//!
//! Callers ask for a capability tier with a structured input and receive structured output with
//! accounting. See docs/architecture/target-architecture.md §4.4.

pub mod adapter;
pub mod adapters;
pub mod builder;
pub mod classify;
pub mod error;
pub mod redact;
pub mod request_hash;
pub mod retry;
pub mod schema_strict;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod types;

pub use adapter::{ProviderAdapter, ProviderRequest, ProviderResponse};
pub use builder::{
    estimate_input_tokens, Gateway, GatewayBuilder, ModelGateway, RouteQuery, RouteSource,
    StaticRouter,
};
pub use error::{BudgetKind, Error, GatewayError, PermanentKind, RateScope, Result, TransientKind};
pub use request_hash::request_hash;
pub use retry::{retry, JitterRng, RetryOutcome, RetryPolicy};
pub use schema_strict::check_strict_compatible;
pub use types::*;
