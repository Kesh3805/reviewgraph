# ADR-011 — Mandatory verification between LLM output and publication

**Status:** Accepted · 2026-10-02

## Context
Invariant 2 says LLM output is never published directly. The legacy system gave two pieces of evidence for why:
- The model falsely reported its own work as complete.
- Findings from different models barely overlapped (2 of 14 locations).

## Decision
- **Pipeline.** The eight stages listed in target-architecture §4.3, implemented in `engine/crates/verification`.
  - Each stage is a pure function over `(candidate, VerificationContext)`.
  - It returns `StageOutcome { pass | fail(reason) | inconclusive, evidence[] }`.
- **Where the LLM is allowed.** Stages 1–5 and the deterministic part of stage 6 make no LLM calls.
  - The VERIFIER model is consulted only on counter-evidence that has already been gathered.
  - It gets a closed question: "Does this evidence refute the claim? Cite the item."
  - It cannot add new claims.
- **Confidence is computed,** never taken from the model:

  ```
  c = clamp(0, 1,
        0.25*anchor + 0.20*deterministic + 0.20*graph + 0.15*repo
      + 0.10*reproduction + 0.10*agreement
      - 0.35*contradiction - 0.15*inference_uncertainty)
  ```

  The weights live in one table, versioned as `verification_version`, and are calibrated on the benchmark.
- **Base/head comparison.** The finding's predicate is re-evaluated on the base. If it holds identically there and the impact graph shows no growth in exposure, the finding is marked `SUPPRESSED_PREEXISTING`.
- **Persistence.** Every candidate is persisted with its lifecycle state and suppression reason.
- **Stage 1 reuses legacy gates.** The legacy prototype's gates are re-implemented from the specification in audit §8, with tests:
  - the file exists
  - the line is valid
  - evidence is present
  - latent findings never block

## Alternatives
| Option | Rejected because |
|---|---|
| Ask the same model "are you sure?" | PRD §146 R4 rejects it: it adds no independent evidence. |
| Accept above a model-reported confidence | Self-reported confidence is uncalibrated. |

## Consequences
- Reviewers must cite structured evidence: symbol keys, ranges and the relations they claim. The reviewer output schema enforces this. The types live in [`engine/crates/review-core/src/evidence.rs`](../../engine/crates/review-core/src/evidence.rs); the "at least one strong evidence source" rule is `has_strong_evidence`, which counts only *effective* strength (model-claimed evidence is capped until verification confirms it).
- Verification depth scales with risk.
