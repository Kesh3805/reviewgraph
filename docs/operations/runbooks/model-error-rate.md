# Model error rate above 5 percent

Alert: `rg-model-error-rate` (severity ticket). Dashboard: `rg-model-usage-cost` (see `infra/openobserve/dashboards`).

## Symptom

Model error rate above 5 percent. Condition: ratio of llm_requests_total{outcome!="ok"} over 15m, min volume 20 > 0.05 for 15m.

## Impact

Reviews may be delayed, degraded or skipped until resolved.

## Likely causes

- Recent deploy or configuration change.
- Dependency saturation or outage (PostgreSQL, Qdrant, model provider, GitHub).
- Unusual traffic volume.

## Diagnosis

- Open the `rg-model-usage-cost` dashboard and compare against the last 24 h.
- Inspect a slow or failing trace by `span_name` in the traces stream.
- Local: `docker compose logs <service>` for the affected service.
- Check `llm_requests_total` for the breakdown by closed-enum labels.

## Mitigation

Address the cause found above; roll back the last change if the alert began after a deploy. Pause the alert in OpenObserve only for planned maintenance.

## Escalation

Page the on-call engineer if unresolved after 30 minutes (immediately for page severity).

## Related alerts

See the other alerts under `infra/openobserve/alerts` and their runbooks in this directory.
