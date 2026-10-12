# Qdrant p95 above 500 ms

Alert: `rg-qdrant-latency-high` (severity ticket). Dashboard: `rg-qdrant-latency` (see `infra/openobserve/dashboards`).

## Symptom

Qdrant p95 above 500 ms. Condition: p95 of qdrant_query_duration_seconds > 0.5 for 10m.

## Impact

Reviews may be delayed, degraded or skipped until resolved.

## Likely causes

- Recent deploy or configuration change.
- Dependency saturation or outage (PostgreSQL, Qdrant, model provider, GitHub).
- Unusual traffic volume.

## Diagnosis

- Open the `rg-qdrant-latency` dashboard and compare against the last 24 h.
- Inspect a slow or failing trace by `span_name` in the traces stream.
- Local: `docker compose logs <service>` for the affected service.
- Check `qdrant_query_duration_seconds` for the breakdown by closed-enum labels.

## Mitigation

Address the cause found above; roll back the last change if the alert began after a deploy. Pause the alert in OpenObserve only for planned maintenance.

## Escalation

Page the on-call engineer if unresolved after 30 minutes (immediately for page severity).

## Related alerts

See the other alerts under `infra/openobserve/alerts` and their runbooks in this directory.
