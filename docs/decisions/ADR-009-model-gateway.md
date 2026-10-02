# ADR-009 — Provider-neutral Model Gateway as an in-engine library

**Status:** Accepted · 2026-10-02

## Context
The legacy system was bound to one CLI and one model. The PRD and the stack mandate require provider neutrality, and they forbid SDK calls scattered across reviewers.

## Decision
- **Placement.** `engine/crates/model-gateway` is a library used by reviewers and verification. In the MVP it is not a separate network service, which removes one network hop and one deployable. Its `ModelGateway` trait is shaped so it can move behind gRPC unchanged later.
- **Contract** (see target-architecture §4.4):
  - Request: `ModelRequest { task, tier, reasoning, input, output_schema, max_output_tokens, cache, privacy, budget, trace }`
  - Response: `ModelResponse { output, usage, latency, provider, model, cached_tokens, cost, request_hash }`
- **Adapters:**
  - `anthropic`: Messages API, structured output through a forced tool with a JSON schema, `cache_control` prompt caching.
  - `openai`: Responses API with a `json_schema` text format.
  - `replay`: deterministic fixtures for offline tests and benchmarks.
- **Cross-cutting behaviour lives in the gateway, once.** Adapters are thin HTTP mappings. The gateway owns:
  - timeouts and retries
  - rate limits and budgets
  - output schema validation
  - accounting
  - redaction
  - tracing
- **No reviewer constructs a provider client.** `reviewers` depends only on the trait. A `cargo deny` ban on `reqwest` in `reviewers` and `verification` enforces this.

## Alternatives
| Option | Rejected because |
|---|---|
| Separate gateway service now | An extra deployable with no consumer outside the engine yet. |
| Use provider SDKs directly | No official Rust SDKs, and it would scatter concerns across reviewers. |

## Consequences
- Every `reviewer_runs` row persists `prompt_version`, `reviewer_version`, `model` and `provider`.
- Output that fails schema validation is retried once with a repair instruction. If it fails again it is counted as `structured_output_failure`.
