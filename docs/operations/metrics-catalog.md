# Metrics catalog

Contract for OBS-005. Names are lower_snake_case with unit suffixes; labels come from closed
enums (`reviewer_type, model, provider, task, tier, queue, outcome, stage, language, severity,
category, state`). Identifiers (organization, repository, run, commit, path) are never labels.
Duration histogram buckets (seconds): 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60, 120, 300, 900.

| Metric | Type | Unit | Labels |
| --- | --- | --- | --- |
| `review_duration_seconds` | histogram | s | outcome |
| `incremental_index_duration_seconds` | histogram | s | language |
| `files_reparsed_total` | counter | 1 | language |
| `symbols_changed_total` | counter | 1 | language |
| `graph_nodes_total` | gauge | 1 | language |
| `graph_edges_total` | gauge | 1 | language |
| `graph_invalidations_total` | counter | 1 | stage |
| `context_tokens_total` | counter | 1 | reviewer_type |
| `context_symbols_total` | counter | 1 | reviewer_type |
| `qdrant_queries_total` | counter | 1 | kind, outcome |
| `qdrant_query_duration_seconds` | histogram | s | kind |
| `llm_requests_total` | counter | 1 | provider, model, task, tier, outcome |
| `llm_input_tokens_total` | counter | 1 | provider, model, task, tier |
| `llm_output_tokens_total` | counter | 1 | provider, model, task, tier |
| `llm_cached_tokens_total` | counter | 1 | provider, model, task, tier |
| `llm_cost_estimate` | counter | usd_micros | provider, model, task, tier |
| `candidate_findings_total` | counter | 1 | reviewer_type, category, severity |
| `verified_findings_total` | counter | 1 | reviewer_type, category, severity |
| `published_findings_total` | counter | 1 | reviewer_type, category, severity |
| `suppressed_findings_total` | counter | 1 | reviewer_type, category, severity, reason |
| `finding_acceptance_rate` | gauge | ratio | reviewer_type |
| `finding_false_positive_rate` | gauge | ratio | reviewer_type |
| `queue_depth` | gauge | 1 | queue, state |
| `queue_wait_seconds` | histogram | s | queue |
| `worker_duration_seconds` | histogram | s | queue, outcome |
| `telemetry_export_failures_total` | counter | 1 | |
| `webhook_signature_failures_total` | counter | 1 | |
| `qdrant_scope_violation_total` | counter | 1 | |
| `tenancy_denied_total` | counter | 1 | |
| `trace_context_missing_total` | counter | 1 | queue |
| `log_redactions_total` | counter | 1 | pattern |

## Rate formulas

Over a trailing 7-day window of published findings with any feedback:

- `finding_acceptance_rate = (useful + already_handled) / published_with_feedback`
- `finding_false_positive_rate = false_positive / published_with_feedback`

## Span names

`webhook_received, repository_checkout, repository_index, incremental_graph_update,
diff_analysis, symbol_mapping, impact_analysis, context_selection, qdrant_search,
reviewer_execution, model_request, candidate_generated, finding_verification, deduplication,
publication`.
