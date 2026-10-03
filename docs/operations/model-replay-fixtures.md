# Model replay fixtures

The development environment has no model API keys, so tests, reproducibility checks and the CI
benchmark subset run against recorded or hand-authored model responses (GW-005). The fixture root
is `fixtures/model-replay/` at the repository root (override with `MODEL_REPLAY_DIR`).

## Layout and format

`fixtures/model-replay/{task}/{request_hash[0..2]}/{request_hash}.{provider}.{model}.json`

The `request_hash` is the canonical gateway hash (blake3 over the JCS-encoded request, after
redaction; see `model-gateway/src/request_hash.rs`). It does not depend on provider or model, so
the same logical request has one hash on every route. The JSON Schema of a fixture is
`engine/crates/model-gateway/schemas/replay-fixture.v1.schema.json`; the workspace test
`fixture_schema_validates_all_committed_fixtures` validates every committed file.

## Lookup order

1. exact `(request_hash, provider, model)`;
2. `(request_hash, any, any)`, the form used by synthetic (hand-authored) fixtures;
3. otherwise `Permanent(ReplayMiss)`, which is never fallback-eligible. Misses are appended to
   `target/replay-misses.jsonl` (`request_hash`, `task`, `prompt_version`, `section_names`) so you
   can record or author the fixture.

## Authoring a synthetic fixture

Copy the miss line's `request_hash`, create the file with provider and model both `any`, set
`"synthetic": true`, and fill `response` (`output`, `usage`, `finish_reason`).

## Recording

Record mode wraps a live adapter and needs all of:

- live keys for the routed provider and `MODEL_GATEWAY_MODE=record`;
- `MODEL_GATEWAY_RECORD=1`;
- a request whose `TenantScope.organization_id` is `FIXTURE_ORG_ID` (so fixtures can never be
  recorded from customer repositories).

Only complete, schema-valid outputs without secret patterns are written. Writes are atomic (temp
file plus link), existing fixtures are never overwritten unless `MODEL_GATEWAY_RECORD_OVERWRITE=1`.

## Other variables

| Variable | Meaning |
| --- | --- |
| `MODEL_GATEWAY_MODE` | `replay` (default in tests and CI), `live` (deployed workers) or `record`. |
| `MODEL_REPLAY_DIR` | Fixture root. |
| `REPLAY_LATENCY=recorded` | Sleep the recorded `latency_ms` (for perf runs). |
