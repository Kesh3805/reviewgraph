# Job transport contract (PIPE-001, ADR-012)

The PostgreSQL `jobs` table (migration `20261008100000_jobs.sql`) is the queue shared by the
TypeScript API (`apps/api/src/jobs/pg-job-queue.ts`, API-007) and the Rust workers
(`engine/crates/pipeline/src/jobs/pg.rs`). Neither side imports the other; the SQL below is the
contract and both adapters must keep it identical.

## Operations

| Operation | Contract |
|---|---|
| enqueue | `INSERT ... ON CONFLICT (idempotency_key) DO NOTHING RETURNING id`, then `pg_notify('jobs_' \|\| queue, id)` in the **same transaction** as the business change. A duplicate key returns the existing job, `created = false`, and sends no notification. |
| claim | `UPDATE jobs SET state='running', locked_by=$w, locked_until=now()+lease, attempts=attempts+1 WHERE id = (SELECT id FROM jobs WHERE queue = ANY($q) AND state='queued' AND run_after <= now() ORDER BY priority DESC, created_at FOR UPDATE SKIP LOCKED LIMIT 1)` |
| heartbeat, complete, fail | Fenced by `id AND locked_by AND attempts AND state='running'`. Zero rows means the lease was lost: the handler must stop and the result is ignored. |
| fail (transient) | `queued` with `run_after = now() + min(300 s, 5 s * 2^(attempts-1))` under full jitter; `dead` when `attempts >= max_attempts`. |
| fail (permanent) | `dead` immediately. |
| fail (rate limited) | `queued` at `now() + retry_after`, the attempt is refunded, `rate_limit_requeues` is incremented, `dead` once `rate_limit_requeues + 1 >= 50`. |
| cancel | `state='cancelled'` for `queued` jobs only, selected by queue, `payload->>'review_run_id'` or an idempotency key prefix. |
| release (shutdown) | A worker's `running` jobs return to `queued`, `locked_by = NULL`, the attempt is refunded, and the queue is notified. |
| reap (PIPE-002) | `running` jobs with `locked_until < now()` return to `queued` (retry after 5 s) or go `dead` at `max_attempts`; they are selected with `SKIP LOCKED` in batches of 100. |

## Tenancy

The table has row-level security (`20261008100040_jobs_rls.sql`). Request paths run with
`app.organization_id` and see only their own jobs. Every worker transaction (claim, heartbeat,
complete, fail, release, reap, cancel) opts in with `set_config('app.job_worker', 'on', true)`,
which is transaction-local. Nothing else may set it.

## Notification channel

`jobs_<queue>` carries the job id as its payload. A notification is a hint only: consumers also poll
(default 5 s plus jitter) so delayed jobs and lost notifications are always picked up.

## Payloads

Payloads are JSON objects of ids only, at most 16 KiB (a CHECK constraint enforces size and type).
Schemas live in `packages/contracts`.

See ADR-012 for the decision and the alternatives considered.
