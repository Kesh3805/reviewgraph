# Review budgets (PIPE-006, PRD §90)

Every review run has six budgets. Exhaustion is **degradation, never failure**: the orchestrator
skips the remaining lower-ranked units, records a `BudgetEvent` per skipped unit, and coverage
(PIPE-008) turns those into a `Degraded` report naming the unreviewed regions. Only the deadline
of a *required* stage is fatal.

| Budget | Deployment hard cap | Repository key (`.review/config.yaml`) |
|---|---|---|
| Reviewed symbols | 400 | `budgets.max_symbols` |
| Graph expansion nodes | 5,000 | not configurable |
| Model tokens (input plus output) | 400,000 | not configurable |
| Candidate findings | 50 | not configurable |
| Model calls | 40 | `budgets.max_model_calls` |
| Review latency | 300 s | `budgets.max_review_seconds` |

Resolution order: deployment hard caps, plan defaults, repository overrides. A repository value
above a hard cap is clamped with a config warning; it is not an error, and repository owners
cannot raise a cap.

## Accounting

* `try_reserve(kind, amount)` is a lock-free compare-and-set loop; concurrent reviewers can never
  overshoot a limit, even by one unit. Tokens are reserved as estimated input plus
  `max_output_tokens` before a call, and `commit(reservation, actual)` refunds the difference.
* `call_budget_for` derives the per-call `CallBudget` (never more than the remaining tokens; the
  call deadline is the run deadline). It fails with `BudgetExceeded` when no call or token is left.
* `watch_deadline` cancels the run-level cancellation token when the latency budget is spent.
* `cap_candidates` keeps the best-ranked `max_candidate_findings` candidates and returns the
  excess, which is persisted as `SUPPRESSED_POLICY { rule: "candidate_cap" }`, never dropped.
* Usage and skipped units are persisted in `review_runs.budget_usage` at stage boundaries and
  restored on resume.
