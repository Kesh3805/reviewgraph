# Session handoff — 2026-10-12

Where development stopped on 2026-10-12. Use this as the starting point for the next session. Task status
lives in `docs/planning/tasks/*.md`; this file records branch, PR and CI state and what to do next.

## State of main

- All round-two lanes are integrated. Main CI went fully green at `66f281f`: rust, integration, ts and docker.
- Round-three work landed through pull requests #1 to #9, all merged. Main CI at `74743e0` (the
  merge of #4, the last PR) is fully green: rust (fmt, clippy, test, deny, contracts, integration),
  integration, ts and docker.
- Task status count on main, 346 tasks:
  - 133 done ☑. This includes 15 INIT tasks whose ☑ mark is mojibake in the P03-P05 doc.
  - 107 in progress ◐.
  - 106 not started ☐.

## Round-three pull requests

| PR | Lane branch | Tasks | Outcome |
|---|---|---|---|
| #1 | `lane/r3-tsa` | TSA-005, TSA-007 (TSA-003 status corrected) | merged |
| #2 | `lane/r3-nest` | NEST-001..007 | merged |
| #3 | `lane/r3-chg` | CHG-001, CHG-002 | merged |
| #4 | `lane/r3-api` | API-013, GX-003 | merged |
| #5 | `lane/r3-idx` | IDX-001..003 | merged |
| #6 | `lane/r3-risk` | RISK-002..006, IMP-008, IMP-010 | merged |
| #7 | `lane/r3-pipe` | PIPE-001, PIPE-002, PIPE-005..007 | merged |
| #8 | `lane/r3-ver` | VER-001 | merged |
| #9 | `lane/r3-reviewers` | REV-S-002, REV-T-001, REV-A-001, REV-P-001, REV-M-001 | merged |

Every PR's checks were green except `windows-cli` (see Known CI issues).

Conflicts were resolved by hand while landing:
- #2 combined TSA-005 references and TSA-007 hashing with the NEST framework context in
  `lang-typescript/src/visit/mod.rs`.
- #7 and #8 overlapped in `pipeline`:
  - VER-001's Postgres finding store sits beside the PIPE-005/007 store.
  - Its metrics module is renamed `store/finding_metrics.rs`.
  - `sqlx` is always on.
  - The `pg` feature gates only `opentelemetry` and `verification`.

Main was merged into some PR branches during landing. Those PRs' checks ran against main as it stood
at that time, not against the final main. The green main CI run above covers all of them together.

## Unmerged lane branches (CI red when the session stopped)

These lanes stopped on the account's usage limit before their CI went green. Each branch keeps its
commits. Rebase on main, fix CI, then open a PR.

| Branch | Last commit | Red workflow |
|---|---|---|
| `lane/r3-graph` | GS-006 file GraphStore; CG/SID audit | rust |
| `lane/r3-inc` | INC-001..004 work | rust |
| `lane/r3-ctx` | CTX-002 candidate generators | rust |
| `lane/r3-prof` | PROF-003 GraphView wiring | rust |
| `lane/r3-cli` | CLI-001 schema assertion | rust |
| `lane/r3-obs-sec` | OBS-006 Rust redaction | rust |
| `lane/r3-dev-ci` | CI-008 Dockerfiles, Trivy | docker, bench, windows-cli (rust and integration green) |
| `lane/r3-perf-qb` | PERF-001 synthetic repo generator | rust, bench |
| `lane/r3-lang-py-java` | LANG-PY-001 analyzer | rust |

## Uncommitted work saved to `wip/` branches

When the usage limit stopped them, these lanes had edits that were never committed. The edits were saved
as a single `wip(...)` commit on top of the lane branch. Before continuing, review and split it into
per-task commits.

| Branch | Size | Content |
|---|---|---|
| `wip/r3-hist` | 28 files, ~3.6k lines | HIST-001..004 history ingestion |
| `wip/r3-mp` | 35 files, ~2.6k lines | MP-001/002 multi-provider |
| `wip/r3-cli` | 5 files | CLI-001 tests and snapshot |
| `wip/r3-perf-qb` | 3 files | PERF/QB work in progress |
| `wip/r3-nest` | 1 file | small NEST edit made after the PR branch was pushed |

## Not started

- LANG-GO-001 and LANG-RS-001. This lane was never launched because of the 20-subagent limit.
- E2E-001, and finishing the INV-* stubs. Do these after the lanes above land.
- QB-001 needs real human-labelled pull requests. The tooling can be built, but a person must do the labelling.

## Known CI issues

- `windows-cli` (CI-009) has never passed. It runs on pull requests only. On Windows runners the fixture
  builder fails (`fixture build failed for init-basic`) and the `PoisonError` failures follow from it.
  Round-three PRs were merged past it by decision. Fixing it is the first CI task next session.
- `docker` currently skips image builds because `infra/docker/*.Dockerfile` does not exist on main. The
  Dockerfiles are on `lane/r3-dev-ci`, and that branch's docker job is red.

## Working rules (unchanged)

- No Docker locally. The host has no C compiler, so GitHub Actions is the build machine. `cargo fmt` and
  `cargo metadata` work locally.
- Regenerate `engine/Cargo.lock` with `cargo metadata`. Never hand-edit it.
- Each lane gets its own migration timestamp range. Every table with `organization_id` gets FORCE RLS
  plus a `tenant_isolation` policy.
- Kysely drift: download the `kysely-generated` artifact from the integration run and commit it.
- One commit per task, with the task ID in the message. No AI attribution anywhere. Mark ☑ only when
  the spec is fully met and CI is green.
- Land changes through PRs with green checks.

## Next session plan

1. Fix `windows-cli` (CI-009) and land the `lane/r3-dev-ci` Dockerfiles, so every PR check can go green.
2. Rebase and land the red lanes in dependency order: graph → inc → ctx → prof → cli → obs-sec →
   perf-qb → lang-py-java.
3. Recover the `wip/` branches (hist, mp first), split them into task commits, get CI green and open PRs.
4. Launch LANG-GO-001 and LANG-RS-001, then the remaining VER-002..012, PIPE-003/004/008..011,
   SUP-002, CTX-003..010 and CLI-002..013.
5. Finish E2E-001 and the INV-* stubs last.

## Cleanup pending (not done yet; needs the owner's OK)

- Stale branches: round-one and round-two `lane/*`, `lane/pipe-2`, `fix/main-ci`, `fix/ci-round2`, and
  each round-three lane once it is merged.
- Old agent worktrees under `.claude/worktrees/`.
