//! `model-gateway` crate: the only way reviewers and verification reach a model (ADR-009).
//!
//! Callers ask for a capability tier with a structured input and receive structured output with
//! accounting. See docs/architecture/target-architecture.md §4.4.

pub mod accounting;
pub mod adapter;
pub mod adapters;
pub mod budget;
pub mod builder;
pub mod cache;
pub mod classify;
pub mod error;
pub mod fixture;
pub mod ratelimit;
pub mod redact;
pub mod request_hash;
pub mod retry;
pub mod router;
pub mod schema_strict;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod types;
pub mod validate;

pub use accounting::{LedgerRecord, LedgerSink, MemoryLedger, PriceTable};
pub use adapter::{ProviderAdapter, ProviderRequest, ProviderResponse};
pub use budget::WorstCasePricer;
pub use builder::{
    estimate_input_tokens, Gateway, GatewayBuilder, ModelGateway, RouteQuery, RouteSource,
    StaticRouter,
};
pub use cache::{CacheEntry, MemoryCache, ResponseCache};
pub use error::{BudgetKind, Error, GatewayError, PermanentKind, RateScope, Result, TransientKind};
pub use ratelimit::{
    limiter_from_lookup, LimitRequest, ModelLimits, NoLimit, RateLimiter, TokenBucketLimiter,
};
pub use request_hash::request_hash;
pub use retry::{retry, JitterRng, RetryOutcome, RetryPolicy};
pub use router::{merge_overrides, route, RoutingFile, RoutingTable, TableRouter};
pub use schema_strict::check_strict_compatible;
pub use types::*;
