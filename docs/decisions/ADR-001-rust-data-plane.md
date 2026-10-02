# ADR-001 — Rust data plane, built in Linux containers

**Status:** Accepted · 2026-10-02

## Context
Repository intelligence is CPU- and memory-bound work:
- parsing thousands of files
- million-symbol in-memory graphs
- bounded traversals
- diffing
- ranking

The PRD (§120) targets 100k files and 1M symbols. Rust gives predictable memory, safe concurrency and first-class tree-sitter bindings.

The **host toolchain** cannot build tokio, sqlx, reqwest or tree-sitter. It is `x86_64-pc-windows-gnu` with no C compiler and a broken bundled `dlltool`. This was verified on 2026-10-02 with a probe crate. The same probe builds and runs in `rust:1-bookworm`, with a 2m46s cold build. The probe used tokio, tree-sitter 0.25, tree-sitter-typescript 0.23, sqlx 0.8 with rustls, and reqwest 0.12 with rustls.

## Decision
- All repository intelligence is written in Rust in the `engine/` cargo workspace (crates per target-architecture §2). This covers scanning, parsing, graph, incremental, diff, impact, context, gateway, reviewers, verification and the CLI.
- The canonical build/test environment is the Linux container `rust:1-bookworm`, wrapped by `engine/scripts/cargo.sh`. The script mounts the workspace and keeps the cargo registry and target directory in persistent named volumes.
- CI builds on Linux runners. The native Windows `review.exe` is produced by CI on `windows-latest` (MSVC), not on this host.

## Alternatives
| Option | Rejected because |
|---|---|
| Keep the pure-Rust, subprocess-only constraint | No tree-sitter means no parser. Fatal. |
| Go | Weaker fit with the tree-sitter ecosystem; the stack mandate is Rust. |
| Install MSVC Build Tools on the host | Valid as a developer option, but it needs a user action on the machine. Documented, not required. |

## Tradeoffs
Container builds add I/O latency on Windows bind mounts. Keeping `target/` in a named volume mitigates this.

## Consequences
- Developers run `engine/scripts/cargo.sh test`.
- Nothing in this repository requires a host C toolchain.

## Migration implications
None. The engine is new code.
