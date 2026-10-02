# ADR-010 — Capability-tier model routing, chosen by evaluation data

**Status:** Accepted · 2026-10-02

## Context
The best model differs by task. We need explicit routing on task, risk, cost, latency, context size, privacy and historical performance.

## Decision
- **Tiers.** Reviewers request a tier, never a model.

  | Tier | Used for |
  |---|---|
  | `CLASSIFIER` | intent and change triage |
  | `FAST_REASONER` | summaries |
  | `REVIEW_REASONER` | candidate generation |
  | `VERIFIER` | contradiction adjudication over gathered evidence |
  | `DEEP_REASONER` | rare cross-module cases; gated on risk ≥ critical plus budget |

- **Routing table.** `routing.yaml` maps `(tier, risk_band, privacy_class)` to an ordered fallback list of `{ provider, model, max_context }`. Defaults ship in the repo and can be overridden per organization or repository.
- **Initial defaults are provisional** until the evaluation harness has data. Each tier also has an OpenAI fallback configured.

  | Tier | Provisional default |
  |---|---|
  | CLASSIFIER | `claude-haiku-4-5` |
  | FAST_REASONER | `claude-haiku-4-5` |
  | REVIEW_REASONER | `claude-sonnet-5-5` |
  | VERIFIER | `claude-sonnet-5-5` |
  | DEEP_REASONER | `claude-opus-5-5` |

- **Changing a default requires an evaluation report.** The report covers precision, recall, FP rate, latency, tokens, cost and structured-output success on the benchmark corpus. It is stored in `model_eval_results` and committed under `benchmarks/quality/reports/`.
- **Privacy class `no_external`** routes only to providers marked `self_hosted`. There are none in the MVP, so the gateway fails closed with `NoEligibleProvider`.

## Alternatives
Hard-coding one model everywhere. That was the legacy behaviour, and the PRD rejects it.

## Consequences
- The router is a pure function and is unit-tested.
- Every routing decision is recorded on the span and the `reviewer_runs` row.
