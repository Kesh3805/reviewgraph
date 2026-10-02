# Error handling

Governing task: DOM-002. Code: `review_core::error` (`CoreError`, `ErrorClass`, `Classify`) and one `error.rs` per library crate.

Typed errors exist to drive three decisions: whether a failure is retried, which HTTP status the engine returns, and which `FAILED_*` state a review run enters. The class of an error, not its message, makes those decisions.

## Rules

1. Library crates never expose `anyhow`, `Box<dyn Error>` or `String` errors. The workspace test `no_library_depends_on_anyhow` (`engine/xtask`) fails if a library crate lists `anyhow` as a dependency.
2. Every library error implements `Classify`, returning an `ErrorClass`.
3. Every public error enum is `#[non_exhaustive]`.
4. Add context with `#[error("...: {source}")]` variants that carry the source error, not by formatting it into a string.
5. Apps convert to `anyhow` only at the boundary: `main`, the top of a job handler, an HTTP handler.
6. Never log an error's `Display` if it may contain source code or secrets. Log the variant name plus identifiers.

Every library error type is `Send + Sync + 'static` so it can cross tokio tasks. `CoreError::InvalidRepoPath` echoes the rejected path capped at 256 characters.

## Error classes

`ErrorClass` has a stable snake_case wire form, which is also the `error.class` span attribute, the `error_class` metric label and the allowed values of `review_runs.failure_class`.

| ErrorClass | HTTP (review-engine/API) | Job outcome (PIPE-001) | Run state effect |
|---|---|---|---|
| `invalid_input` | 400 | `failed`, no retry | `FAILED_<stage>` |
| `not_found` | 404 | `failed`, no retry | `FAILED_<stage>` |
| `conflict` | 409 | no retry; the compare-and-swap was lost, so the stage aborts quietly (it is superseded or done elsewhere) | unchanged |
| `transient` | 503 | retry with jittered backoff until `max_attempts`, then `dead` | `FAILED_<stage>` only when dead |
| `rate_limited` | 429 (`Retry-After`) | retry at `retry_after` | as for `transient` |
| `permanent` | 422 | `failed` | `FAILED_<stage>` |
| `cancelled` | 409 | `cancelled` | `CANCELLED` / `SUPERSEDED` |
| `internal` | 500 | `failed`, alert | `FAILED_<stage>` |

`ErrorClass::is_retryable` (true only for `transient` and `rate_limited`) is the single input to the automatic-retry decision. `conflict` is never retried: retrying a lost compare-and-swap would re-execute a stage that someone else has already advanced.

## Per-crate pattern

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] review_core::CoreError),
    // domain variants are added by the crate's own tasks
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl review_core::Classify for Error { /* exhaustive match */ }
```
