# ADR-002 — TypeScript/NestJS control plane

**Status:** Accepted · 2026-10-02

## Context
The control plane covers:
- tenancy, auth and onboarding
- the GitHub App and webhooks
- review lifecycle and publication
- feedback and admin APIs

This is network-bound CRUD plus provider integration. The provider SDK ecosystem is strongest in TypeScript: Octokit, `@octokit/webhooks` and GitHub App auth.

## Decision
- `apps/api` is NestJS 11 on Node 24, with TypeScript in strict mode.
- Database access uses `pg` plus Kysely, a typed query builder.
- Schema migrations are owned by `engine/migrations` (ADR-014). The API never mutates schema.
- Kysely types are generated from the migrated database (`kysely-codegen`). CI fails if the generated types drift.
- Tests use Jest.
- Providers sit behind `RepositoryProvider` / `ReviewPublisher` ports.

## Alternatives
| Option | Rejected because |
|---|---|
| Rust (Axum) for everything | Reinvents GitHub App tooling and slows iteration on product CRUD. |
| Prisma | Owns migrations and a schema DSL, which conflicts with one SQL migration source shared with Rust. |
| Express without Nest | Gives less structure (modules, guards, DI) at this size. |

## Tradeoffs and consequences
- Two languages, coupled through three things: PostgreSQL rows, the PG job queue (ADR-012), and the review-engine HTTP API.
- Shared contracts are generated from Rust types: `schemars` produces JSON Schema, and `json-schema-to-typescript` produces the TS types.

## Migration implications
None. This is a greenfield repository. The legacy prototype's dashboard and `gh`-CLI transport are not carried over (audit §7).
