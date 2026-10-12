//! Repository indexing building blocks (phase 7): configuration, versions, progress,
//! parse tolerance and the parse cache.

pub mod config;
pub mod parse_cache;
pub mod progress;
pub mod tolerance;
pub mod versions;

pub use config::IndexConfig;
pub use parse_cache::{
    CacheLookup, InMemoryParseCache, MissReason, ParseCache, ParseCacheError, ParseCacheKey,
};
pub use progress::{IndexPhase, NoopProgress, ProgressSink, ProgressThrottle};
pub use tolerance::{FileOutcomeKind, OutcomeCounts, ToleranceExceeded, TolerancePolicy};
pub use versions::{compute_versions, ConfigComponents, VersionInputs};
