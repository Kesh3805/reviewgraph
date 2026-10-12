//! Logging and optional telemetry for CLI runs (CLI-001).
//!
//! Logs go to stderr only: stdout carries command results. Telemetry export is off unless
//! `REVIEW_OTEL=1`; then the shared `telemetry` crate installs OTLP export (endpoint from
//! `OTEL_EXPORTER_OTLP_ENDPOINT`) and keeps its pretty log layer on stderr.

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

use crate::cli::ColorMode;

/// Keeps the OTLP pipeline alive; flushes it on drop.
#[derive(Debug, Default)]
pub struct LogGuard {
    _telemetry: Option<telemetry::TelemetryGuard>,
}

/// `EnvFilter` directives for the `-v`/`-q` counters: warn by default, `-q` errors only,
/// `-v` info, `-vv` debug, `-vvv` trace.
pub fn directives(verbose: u8, quiet: u8) -> &'static str {
    match i16::from(verbose) - i16::from(quiet) {
        i if i <= -1 => "error",
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    }
}

/// True when `REVIEW_OTEL` asks for telemetry export.
pub fn otel_requested(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v == "1" || v.eq_ignore_ascii_case("true"))
}

/// Installs the process-wide subscriber. Never fails: a broken telemetry setup degrades to
/// plain stderr logging with a warning.
pub fn init(verbose: u8, quiet: u8, color: ColorMode) -> LogGuard {
    let level = directives(verbose, quiet);
    let rust_log_set = std::env::var_os("RUST_LOG").is_some_and(|v| !v.is_empty());

    if otel_requested(std::env::var("REVIEW_OTEL").ok().as_deref()) {
        match telemetry::TelemetryConfig::from_env_with_format(
            "review-cli",
            telemetry::LogFormat::Pretty,
        ) {
            Ok(config) => {
                let config = if rust_log_set {
                    config
                } else {
                    config.with_filter(level)
                };
                match telemetry::init(config) {
                    Ok(guard) => {
                        return LogGuard {
                            _telemetry: Some(guard),
                        }
                    }
                    Err(error) => eprintln!("warning: telemetry disabled: {error}"),
                }
            }
            Err(error) => eprintln!("warning: telemetry disabled: {error}"),
        }
    }

    let filter = if rust_log_set {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level))
    } else {
        EnvFilter::new(level)
    };
    let ansi = match color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
        }
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(ansi)
        .with_target(false)
        .try_init();
    LogGuard::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_maps_to_directives() {
        assert_eq!(directives(0, 0), "warn");
        assert_eq!(directives(1, 0), "info");
        assert_eq!(directives(2, 0), "debug");
        assert_eq!(directives(5, 0), "trace");
        assert_eq!(directives(0, 1), "error");
        assert_eq!(directives(1, 1), "warn");
    }

    #[test]
    fn otel_is_opt_in() {
        assert!(!otel_requested(None));
        assert!(!otel_requested(Some("0")));
        assert!(otel_requested(Some("1")));
        assert!(otel_requested(Some("TRUE")));
    }
}
