//! Environment-driven telemetry configuration.

use std::fmt;

use crate::error::{Error, Result};

/// A string that never appears in `Debug` or `Display` output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The only way to read the value; call sites are the exporter builders.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// Stdout log format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    #[default]
    Json,
    Pretty,
}

/// Process-wide telemetry configuration. Build with [`TelemetryConfig::from_env`] or
/// [`TelemetryConfig::new`].
#[derive(Clone)]
pub struct TelemetryConfig {
    pub service_name: String,
    pub service_version: String,
    pub environment: String,
    pub git_sha: Option<String>,
    /// Base OTLP endpoint, e.g. `http://127.0.0.1:25080/api/default`. `None` means no network.
    pub endpoint: Option<String>,
    /// OTLP request headers (values are secrets).
    pub headers: Vec<(String, Secret)>,
    pub otel_enabled: bool,
    pub log_format: LogFormat,
    /// `EnvFilter` directives (the value of `RUST_LOG`).
    pub filter: String,
    /// Trace id ratio in `0.0..=1.0`.
    pub sampler_ratio: f64,
}

impl fmt::Debug for TelemetryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelemetryConfig")
            .field("service_name", &self.service_name)
            .field("service_version", &self.service_version)
            .field("environment", &self.environment)
            .field("git_sha", &self.git_sha)
            .field("endpoint", &self.endpoint)
            .field("headers", &self.headers)
            .field("otel_enabled", &self.otel_enabled)
            .field("log_format", &self.log_format)
            .field("filter", &self.filter)
            .field("sampler_ratio", &self.sampler_ratio)
            .finish()
    }
}

impl TelemetryConfig {
    /// A stdout-only JSON configuration for `service_name`.
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            service_version: env!("CARGO_PKG_VERSION").to_owned(),
            environment: "development".to_owned(),
            git_sha: None,
            endpoint: None,
            headers: Vec::new(),
            otel_enabled: true,
            log_format: LogFormat::Json,
            filter: "info".to_owned(),
            sampler_ratio: 1.0,
        }
    }

    pub fn with_service_version(mut self, version: impl Into<String>) -> Self {
        self.service_version = version.into();
        self
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), Secret::new(value)));
        self
    }

    pub fn with_log_format(mut self, format: LogFormat) -> Self {
        self.log_format = format;
        self
    }

    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = filter.into();
        self
    }

    /// Reads the process environment. See `docs/operations/observability.md`.
    pub fn from_env(default_service_name: &str) -> Result<Self> {
        Self::from_lookup(default_service_name, |k| std::env::var(k).ok())
    }

    /// [`from_env`](Self::from_env) with a different stdout format when `RG_LOG_FORMAT` is
    /// unset (the CLI defaults to `pretty`).
    pub fn from_env_with_format(
        default_service_name: &str,
        default_format: LogFormat,
    ) -> Result<Self> {
        Self::from_lookup_with(default_service_name, default_format, |k| {
            std::env::var(k).ok()
        })
    }

    /// Like [`from_env`](Self::from_env) with an injectable variable lookup.
    pub fn from_lookup(
        default_service_name: &str,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        Self::from_lookup_with(default_service_name, LogFormat::Json, lookup)
    }

    fn from_lookup_with(
        default_service_name: &str,
        default_format: LogFormat,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let get = |k: &str| {
            lookup(k)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let mut cfg =
            Self::new(get("OTEL_SERVICE_NAME").unwrap_or_else(|| default_service_name.to_owned()));
        cfg.log_format = default_format;
        cfg.endpoint = get("OTEL_EXPORTER_OTLP_ENDPOINT");
        if let Some(raw) = get("OTEL_EXPORTER_OTLP_HEADERS") {
            cfg.headers = parse_headers(&raw)?;
        }
        if let Some(v) = get("RUST_LOG") {
            cfg.filter = v;
        }
        if let Some(v) = get("RG_LOG_FORMAT") {
            cfg.log_format = match v.to_ascii_lowercase().as_str() {
                "json" => LogFormat::Json,
                "pretty" => LogFormat::Pretty,
                _ => {
                    return Err(Error::InvalidConfig {
                        field: "RG_LOG_FORMAT",
                        reason: "expected `json` or `pretty`".into(),
                    })
                }
            };
        }
        if let Some(v) = get("RG_OTEL_ENABLED") {
            cfg.otel_enabled = match v.to_ascii_lowercase().as_str() {
                "true" | "1" => true,
                "false" | "0" => false,
                _ => {
                    return Err(Error::InvalidConfig {
                        field: "RG_OTEL_ENABLED",
                        reason: "expected `true` or `false`".into(),
                    })
                }
            };
        }
        if let Some(v) = get("OTEL_TRACES_SAMPLER_ARG") {
            cfg.sampler_ratio = v.parse().map_err(|_| Error::InvalidConfig {
                field: "OTEL_TRACES_SAMPLER_ARG",
                reason: "expected a number between 0 and 1".into(),
            })?;
        }
        if let Some(v) = get("RG_ENV") {
            cfg.environment = v;
        }
        cfg.git_sha = get("RG_GIT_SHA");
        cfg.validate()?;
        Ok(cfg)
    }

    /// Checks everything that would otherwise only fail at first export.
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.sampler_ratio) {
            return Err(Error::InvalidConfig {
                field: "OTEL_TRACES_SAMPLER_ARG",
                reason: "must be within 0.0..=1.0".into(),
            });
        }
        if let Some(endpoint) = &self.endpoint {
            if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
                return Err(Error::InvalidConfig {
                    field: "OTEL_EXPORTER_OTLP_ENDPOINT",
                    reason: "must start with http:// or https://".into(),
                });
            }
        }
        for (name, value) in &self.headers {
            validate_header(name, value)?;
        }
        Ok(())
    }

    /// The endpoint export would use, or `None` when export is off.
    pub fn effective_endpoint(&self) -> Option<&str> {
        if self.otel_enabled {
            self.endpoint.as_deref()
        } else {
            None
        }
    }
}

fn validate_header(name: &str, value: &Secret) -> Result<()> {
    let name_ok = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
    if !name_ok {
        return Err(Error::InvalidConfig {
            field: "OTEL_EXPORTER_OTLP_HEADERS",
            reason: "header name is empty or contains invalid characters".into(),
        });
    }
    if value.expose().bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(Error::InvalidConfig {
            field: "OTEL_EXPORTER_OTLP_HEADERS",
            reason: format!("value of header `{name}` contains control characters"),
        });
    }
    Ok(())
}

/// Parses `k1=v1,k2=v2`. Values may contain `=` (base64 padding) and `%XX` escapes.
fn parse_headers(raw: &str) -> Result<Vec<(String, Secret)>> {
    let mut out = Vec::new();
    for pair in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, value) = pair.split_once('=').ok_or_else(|| Error::InvalidConfig {
            field: "OTEL_EXPORTER_OTLP_HEADERS",
            reason: "expected comma separated name=value pairs".into(),
        })?;
        out.push((
            name.trim().to_owned(),
            Secret::new(percent_decode(value.trim())),
        ));
    }
    Ok(out)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(vars: &[(&str, &str)]) -> Result<TelemetryConfig> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        TelemetryConfig::from_lookup("svc", move |k| {
            vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
        })
    }

    #[test]
    fn defaults_are_stdout_only() {
        let c = cfg(&[]).unwrap();
        assert_eq!(c.service_name, "svc");
        assert!(c.effective_endpoint().is_none());
        assert_eq!(c.log_format, LogFormat::Json);
        assert!((c.sampler_ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_basic_auth_header_with_padding_and_space() {
        let c = cfg(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://h:1/api/default"),
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "Authorization=Basic%20YWJj==, x-a=b",
            ),
        ])
        .unwrap();
        assert_eq!(c.headers.len(), 2);
        assert_eq!(c.headers[0].1.expose(), "Basic YWJj==");
    }

    #[test]
    fn debug_output_never_prints_auth_header() {
        let c = cfg(&[(
            "OTEL_EXPORTER_OTLP_HEADERS",
            "Authorization=Basic supersecret",
        )])
        .unwrap();
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("supersecret"), "{dbg}");
        assert!(dbg.contains("REDACTED"));
    }

    #[test]
    fn bad_header_fails_at_startup() {
        assert!(cfg(&[("OTEL_EXPORTER_OTLP_HEADERS", "no-equals")]).is_err());
        assert!(cfg(&[("OTEL_EXPORTER_OTLP_HEADERS", "bad name=v")]).is_err());
    }

    #[test]
    fn bad_values_rejected() {
        assert!(cfg(&[("OTEL_TRACES_SAMPLER_ARG", "2")]).is_err());
        assert!(cfg(&[("RG_LOG_FORMAT", "xml")]).is_err());
        assert!(cfg(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "localhost:4318")]).is_err());
    }

    #[test]
    fn disabled_flag_hides_endpoint() {
        let c = cfg(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://h:1"),
            ("RG_OTEL_ENABLED", "false"),
        ])
        .unwrap();
        assert!(c.effective_endpoint().is_none());
    }
}
