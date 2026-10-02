//! `telemetry` crate (OBS-001): one `init()` for every Rust binary.
//!
//! Structured JSON logs go to stdout; when `OTEL_EXPORTER_OTLP_ENDPOINT` is set, traces,
//! metrics and logs are also exported over OTLP/HTTP protobuf to OpenObserve. See
//! docs/architecture/target-architecture.md §8 and docs/operations/observability.md.

pub mod attrs;
pub mod config;
pub mod error;
pub mod guard;
pub mod init;
pub mod json_format;
pub mod otlp;

pub use config::{LogFormat, Secret, TelemetryConfig};
pub use error::{Error, Result, TelemetryError};
pub use guard::TelemetryGuard;
pub use init::{init, init_with, testing, BoxedLayer};
pub use otlp::export_failures_total;

#[doc(hidden)]
pub mod __private {
    pub use tracing;
}
