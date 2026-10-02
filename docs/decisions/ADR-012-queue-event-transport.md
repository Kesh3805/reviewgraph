# ADR-012 — Job transport: PostgreSQL queue now, NATS JetStream when measured

**Status:** Accepted · 2026-10-02

## Context
- **Producers:** NestJS.
- **Consumers:** Rust workers (index, review) and NestJS (publish).
- **Existing queue:** none. The legacy system has no queue (audit §5), so no BullMQ setup needs preserving.

## Comparison

| Criterion | BullMQ (Redis) | NATS JetStream | PostgreSQL `jobs` + SKIP LOCKED |
|---|---|---|---|
| Delivery | at-least-once | at-least-once, ack/nak | at-least-once via lease + reaper |
| Idempotency | jobId dedup | Msg-Id dedup window | `UNIQUE(idempotency_key)`, in the same transaction as the business rows |
| Consumer groups | per queue | durable consumers | per queue name |
| Retries | built in | max_deliver + backoff | `attempts` + `run_after` |
| Dead letter | failed set | stream/advisory | `state='dead'` |
| **Rust support** | **no maintained client** (Lua-script protocol) | `async-nats` | `sqlx` (already a dependency) |
| TypeScript support | excellent | `nats.js` | `pg` (already a dependency) |
| Operational burden | Redis (already present) | +1 stateful service | none |
| Observability | Bull Board | NATS metrics | queue depth is a SQL query |
| Throughput | very high | very high | ~1–5k jobs/s, far above need (PR events are tens per minute) |
| Latency | ms | ms | ms with `LISTEN/NOTIFY` |
| Enqueue atomic with the state change | no | no | **yes** |

## Decision
- Use the PostgreSQL job queue for the MVP and production v1.
- Put it behind a `JobQueue` port in both languages: Rust `pipeline::jobs` and TS `apps/api/src/jobs`.
- Reject BullMQ: it has no Rust consumer, and a polyglot system is the core requirement.
- Name NATS JetStream as the migration target. Migrate when either of these is measured:
  - sustained throughput above 500 jobs/s
  - job-table bloat or lock contention showing up in PG metrics

## Consequences
- Enqueue happens in the same transaction as the state change. This removes dual-write bugs, such as a webhook being stored with no job created for it.
- Migrating later means replacing two port adapters. Payload schemas already live in `packages/contracts`.
