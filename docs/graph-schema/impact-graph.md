# Impact graph

The impact graph is the derived neighbourhood of a pull request's changed symbols
(target-architecture §3.7, PRD §30–§31). It is produced by `engine/crates/impact`
(`impact::graph`) and stored as a stage output; reviewers, context selection and verification
all read the same value. The JSON Schema is `impact::graph::ImpactGraph::json_schema()`.

## Shape

```
ImpactGraph { schema_version, symbols: [SymbolImpact], budget, stats, flags, test_targets, input_hash }
SymbolImpact { seed, seed_id, side, skipped?, untested, elements: [ImpactElement], truncation: [Truncation] }
ImpactElement { node, node_id, kind, relation, distance, path: [PathStep], min_confidence, alt_paths, weak,
                endpoint?, test?, resource?, touches?, missing_on_head? }
PathStep { from, edge, to, confidence, graph: base|head }
Truncation { relation?, limit, dropped, reason: limit|depth|total_cap|seed_missing, visited? }
```

* Node keys are 32 lowercase hex characters (`blake3(node id)[..16]`, ADR-005).
* Confidences are permille integers (`0..=1000`), the codegraph `Confidence` representation.
* `path` runs from the seed to the element; each step is the edge **as stored** (`from` is the
  edge source), so a verifier re-checks a step with one edge lookup. Interface dispatch is
  recorded as an `IMPLEMENTS` step and does not add distance.
* `min_confidence` is the weakest step of `path`. `weak = min_confidence < budget.min_confidence`;
  weak elements are listed but never expanded further.
* When an element is reached more than once under the same relation the best path wins:
  highest `min_confidence`, then smallest `distance`, then the lexicographically smallest
  sequence of node keys. `alt_paths` counts the others.
* Elements are sorted by `(relation, distance, -min_confidence, node_id)`; seeds by
  `(path, seed id)` of the change model order.
* `input_hash = blake3(change-model hash ‖ head snapshot ‖ base snapshot ‖ budget ‖ IMPACT_VERSION)`.

## Relations

| Relation | Meaning | Graph | Distance |
|---|---|---|---|
| `caller` | reverse `CALLS`, plus callers of the interface member the seed implements | head (base for removed seeds) | 1–2 (3 when the budget allows) |
| `callee` | forward `CALLS` | head | 1 |
| `removed_callee` | a call the change removed (CHG-003), resolved on base | base | 1 |
| `interface` | the interface member the seed implements | head | 1 |
| `implementation` | a sibling implementation of the same interface member, or an implementation of an interface-member seed | head | 2 / 1 |
| `override` / `overridden_by` | `OVERRIDES` out / in | head | 1 |
| `subtype` / `supertype` | reverse / forward `EXTENDS`, `IMPLEMENTS` of a type seed | head | ≤ 2 |
| `related_type` | reverse `USES_TYPE`, `ACCEPTS_TYPE`, `RETURNS_TYPE` of a type seed (a DTO pulls in its handlers) | head | 1 |
| `endpoint` | a public entrypoint reached by reverse calls: `http` (via `HANDLED_BY`), `queue`, `cli`, `cron`, `worker` | head | ≤ 7 |
| `test` | a test case mapped to the seed (see below) | head | 1–3 |
| `config` / `env_var` | `READS_CONFIG`, `WRITES_CONFIG` targets; other readers of an env var | head / base | 1 / 2 |
| `db_table` | `READS_TABLE`, `WRITES_TABLE` targets; other writers and readers of a table | head / base | 1 / 2 |
| `db_entity` | an ORM entity injected into the seed's container class | head | 1 |
| `queue_producer` | a queue the seed produces to (`PRODUCES_JOB`, `PUBLISHES`) | head / base | 1 |
| `queue_consumer` | a queue the seed consumes, or a consumer of a queue it produces to | head | 1 / 2 |
| `external_api` | `DEPENDS_ON` an external API or dependency | head / base | 1 |
| `container` | reserved for the enclosing class/module | head | 1 |

Resource elements carry `resource { role, removed }`; `removed = true` marks an edge that exists
on base only. Callers carry `touches`, the tables and queues the caller itself reads or writes,
so a caller's table is visible on the caller instead of being duplicated as a seed relation.

## Budgets

`ImpactBudget` caps every expansion; every stop is a `Truncation` and is aggregated into
`stats.truncations`. Defaults:

| Field | Default |
|---|---|
| `max_caller_depth` | 2 |
| `max_callers` | 50 |
| `max_callees` | 30 |
| `max_type_relations` | 30 |
| `max_endpoints` | 10 |
| `max_endpoint_depth` / `max_endpoint_visits` | 6 / 2,000 |
| `max_tests` | 20 |
| `max_resources` / `max_resource_other_side` | 30 / 5 |
| `max_total_elements_per_symbol` | 200 |
| `max_total_elements_pr` | 5,000 |
| `min_confidence` | 0.5 |
