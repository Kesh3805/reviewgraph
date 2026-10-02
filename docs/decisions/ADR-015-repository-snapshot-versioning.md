# ADR-015 — Repository snapshot and intelligence versioning

**Status:** Accepted · 2026-10-02

## Context
Derived data must be reproducible (PRD §15, §105, §121). It must also be selectively invalidated: a version bump should invalidate only the layers it affects.

## Decision

### Provenance
Every derived artifact records:
- `commit_sha`
- `graph_schema_version`: an integer, `codegraph::SCHEMA_VERSION`
- `analyzer_versions`: a map of language to semver
- `config_hash`: a hash of the normalized `.review/config.yaml` plus the relevant tsconfig set
- `profile_version`

Model-derived artifacts also record:
- `embedding_space`
- `reviewer_version`
- `prompt_version`
- `provider`
- `model`
- `verification_version`

The types and the invalidation table are implemented in [`engine/crates/review-core/src/provenance.rs`](../../engine/crates/review-core/src/provenance.rs) (`Provenance::invalidation_against`).

### Repository fingerprint (PRD §15)
```
fingerprint = blake3(repository_id ‖ commit_sha ‖ analyzer_versions ‖ graph_schema_version
                     ‖ config_hash ‖ parser_versions ‖ profile_version)
```
This is the cache-validity key of a snapshot.

### Snapshot kinds
- `full`: written on the initial index and when compacting the default branch.
- `delta`: written for PR heads and for default-branch updates.

### What each version bump invalidates
| Bump | Effect |
|---|---|
| `graph_schema_version` | Forces a full rebuild. |
| Analyzer version | Re-parses only that language's files. |
| Model, prompt, or verification version | Invalidates only the model-derived layers (PRD §73). |

### Reproducibility test
Run the pipeline twice under the `replay` provider, using the same fingerprint, config and versions. The test asserts that both runs produce identical `ContextPackage` hashes and identical findings.

## Consequences
- `review status` and the API expose the fingerprint and every recorded version.
- Cache keys include these versions explicitly.
