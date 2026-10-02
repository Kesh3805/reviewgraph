# ADR-013 — OpenTelemetry everywhere, OpenObserve as the single backend

**Status:** Accepted · 2026-10-02

## Context
We need one place for logs, traces, metrics, dashboards and alerts, without running Grafana, Prometheus, Loki, Tempo and Jaeger side by side.

## Decision

### Instrumentation
| Component | Stack |
|---|---|
| Rust | `tracing`, `tracing-subscriber` (JSON output), `tracing-opentelemetry`, `opentelemetry-otlp` (HTTP/protobuf) |
| NestJS | `@opentelemetry/sdk-node`, with auto-instrumentation for http, pg and ioredis, plus manual spans for review stages |

### Export
- Telemetry goes over OTLP directly to OpenObserve at `/api/{org}/v1/{traces,metrics,logs}`.
- There is no collector in the MVP. Add one when tail sampling or fan-out is needed.

### Trace continuity
- The webhook span's `traceparent` is stored on each job row, and consumers restore it.
- As a result, one PR review is one trace.

### Conventions
- Correlation attributes and span names are defined in target-architecture §8.
- Metric names come from the master plan's OBS tasks.

### Dashboards and alerts
These are versioned JSON files in `infra/openobserve/`. A script applies them through the OpenObserve API.

### Redaction
- A `tracing` layer and a NestJS log formatter redact token patterns, auth headers and secret assignments.
- Source code and prompts are never logged.

## Alternatives
| Option | Rejected because |
|---|---|
| Grafana + Prometheus + Loki + Tempo | Four systems; explicitly not wanted. |
| A vendor APM | Cost, and customer source context would leave the boundary. |

## Consequences
- Every service reads `OTEL_EXPORTER_OTLP_ENDPOINT` and an auth header.
- Telemetry stays local-only by default.
