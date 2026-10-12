# OpenObserve assets as code

- `dashboards/*.json`: one dashboard per file, stable `dashboardId` (`rg-<slug>`).
- `alerts/*.json`, `destinations/*.json`, `templates/*.json`: alert definitions (OBS-008).
- `apply.mjs`: idempotent upsert. `--dry-run` prints the plan, `--prune` removes `rg-` items with no file, `--allow-remote` is required for non-loopback endpoints.
- `validate.mjs`: schema and catalog checks (metric names must exist in `docs/operations/metrics-catalog.md`; every alert has a runbook and vice versa).

Environment: `RG_OO_URL` (default `http://localhost:25080`), `RG_OO_EMAIL`, `RG_OO_PASSWORD`. Credentials are never printed or written.

Run: `pnpm obs:apply` (add `-- --dry-run` to preview), `node infra/openobserve/validate.mjs`, `node --test infra/openobserve/apply.test.mjs`.

| Dashboard | Source metrics |
| --- | --- |
| review-latency | `review_duration_seconds`, trace stage durations |
| index-latency | `incremental_index_duration_seconds`, `files_reparsed_total` |
| model-usage-cost | `llm_*` |
| findings-funnel | `candidate_findings_total`, `verified_findings_total`, `published_findings_total`, `suppressed_findings_total` |
| false-positive-feedback | `finding_acceptance_rate`, `finding_false_positive_rate` |
| worker-health | `worker_duration_seconds`, `telemetry_export_failures_total` |
| qdrant-latency | `qdrant_query_duration_seconds`, `qdrant_queries_total` |
| postgres-latency | `queue_wait_seconds`, `graph_invalidations_total` |
| queue-depth | `queue_depth`, `queue_wait_seconds` |
