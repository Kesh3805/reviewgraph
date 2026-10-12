# Local development

## Model gateway environment

The model gateway (`engine/crates/model-gateway`) registers a provider only when its key is set.
No test needs a key: tests use the replay adapter or mocked HTTP.

| Variable | Meaning |
| --- | --- |
| `ANTHROPIC_API_KEY` | Enables the Anthropic adapter (`claude-haiku-4-5`, `claude-sonnet-5-5`, `claude-opus-5-5`). Read once at startup; never logged. |
| `ANTHROPIC_BASE_URL` | Optional host override (used by wiremock tests). Must be `https://` when `RG_ENV=production`. |
| `OPENAI_API_KEY` | Enables the OpenAI Responses adapter (strict `json_schema` output, `store: false`). Candidate model ids come from `routing.yaml`. |
| `OPENAI_BASE_URL` | Optional host override; same https-in-production rule. |

A live smoke command (`review-cli model smoke --provider anthropic`) is not implemented yet; use
the wiremock suite (`engine/scripts/cargo.sh test -p model-gateway`) for adapter verification.

## Model gateway rate limiting

| Variable | Meaning |
| --- | --- |
| `REDIS_URL` | Shared token buckets across workers (dev: `redis://127.0.0.1:26379`, from the build container `redis://host.docker.internal:26379`). Unset means process-local buckets. |
| `RG_WORKER_COUNT_HINT` | Divisor for the local fallback buckets when Redis is unreachable (default 4). |

Limits are configured per model in `routing.yaml` (`providers.<p>.limits`). Tests that need Redis
are behind the `integration` feature: `engine/scripts/cargo.sh test -p model-gateway --features integration`.

## Semantic retrieval (embeddings and Qdrant)

| Variable | Meaning |
| --- | --- |
| `SEMANTIC_EMBEDDING_PROVIDER` | `hash` (default: deterministic, offline, no key), `openai`, or `voyage` (recommended in production). |
| `SEMANTIC_EMBEDDING_MODEL` / `SEMANTIC_EMBEDDING_DIMS` | Provider defaults: `text-embedding-3-small`/1536, `voyage-code-3`/1024, hash 768. |
| `SEMANTIC_EMBEDDING_CONCURRENCY` | Parallel requests per provider instance (default 4). |
| `SEMANTIC_EMBEDDING_VERSION` | Re-embed epoch (default 1); bumping it creates a new collection. |
| `VOYAGE_API_KEY` / `OPENAI_API_KEY` | Needed only by the matching provider. A missing key fails provider construction; the worker then runs without semantic sync and reviews use structural context only. |
| `QDRANT_URL` / `QDRANT_API_KEY` | Qdrant REST endpoint (CI integration job: `http://127.0.0.1:36333`). |

`hash` is the default in development and tests: no text leaves the machine and vectors are
bit-exact across runs. A repository with privacy `no_external` always uses `hash`. Collections are
managed with `review semantic collections|activate|retire`; see
[semantic-reembed.md](semantic-reembed.md). Qdrant tests are behind the `semantic` crate's
`integration` feature and need `QDRANT_URL`.
