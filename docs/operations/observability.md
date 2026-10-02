# Observability

Every Rust binary (`review-cli`, `review-worker`, `review-engine`) calls `telemetry::init` once at
startup (OBS-001). It installs JSON logs on stdout and, when an OTLP endpoint is configured,
exports traces, metrics and logs over OTLP/HTTP protobuf to OpenObserve
(`{endpoint}/v1/traces`, `/v1/metrics`, `/v1/logs`). See ADR-013 and
`docs/architecture/target-architecture.md` section 8.

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset | Base OTLP URL, for example `http://127.0.0.1:25080/api/default` (inside the dev container `http://host.docker.internal:25080/api/default`). Unset means stdout only and no network. |
| `OTEL_EXPORTER_OTLP_HEADERS` | unset | Comma separated `name=value` pairs, for example `Authorization=Basic <base64>`. Values may be `%XX` encoded. Never logged. |
| `OTEL_SERVICE_NAME` | binary name | `service.name` resource attribute. |
| `OTEL_TRACES_SAMPLER_ARG` | `1.0` | Trace id ratio, `0.0` to `1.0`. |
| `RUST_LOG` | `info` | `EnvFilter` directives. |
| `RG_LOG_FORMAT` | `json` (`pretty` for `review-cli`) | `json` or `pretty`. |
| `RG_OTEL_ENABLED` | `true` | `false` forces stdout only even when an endpoint is set. |
| `RG_ENV` | `development` | `deployment.environment` resource attribute. |
| `RG_GIT_SHA` | unset | `git.sha` resource attribute. |

An invalid value (bad header string, sampler ratio out of range, unknown format) fails startup with
a typed error. An unreachable endpoint never fails or blocks the process: failures are counted in
`telemetry_export_failures_total` and reported to stderr at most once per minute.

## JSON log line

`{timestamp, level, target, message, fields, span, spans, trace_id, span_id}` plus the correlation
attributes (`request_id, review_run_id, repository_id, organization_id, pull_request_id,
commit_sha, job_id, reviewer_type, candidate_finding_id`) flattened to the top level from the
enclosing spans. Create such spans with `telemetry::correlation_span!`.

## Verifying export locally

```
OTEL_EXPORTER_OTLP_ENDPOINT=http://host.docker.internal:25080/api/default \
OTEL_EXPORTER_OTLP_HEADERS="Authorization=Basic <base64 of user:password>" \
  engine/scripts/cargo.sh run -p telemetry --example emit
```
