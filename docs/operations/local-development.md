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
