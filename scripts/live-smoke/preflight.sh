#!/usr/bin/env bash
# Live-smoke preflight (E2E-002). Prints `set` or `missing` per variable, never a value.
#
# Reads LIVE_SMOKE_ENV_FILE (default .env.live) if present. Exit codes:
#   0 all checks passed, 1 a variable is missing, 2 tunnel unreachable, 3 model key rejected.
# Test seams: LIVE_SMOKE_SKIP_MODEL_CALL=1 skips the one-token model call.
set -u

env_file="${LIVE_SMOKE_ENV_FILE:-.env.live}"
if [ -f "$env_file" ]; then
  set -a
  # shellcheck disable=SC1090
  . "$env_file"
  set +a
fi

status=0

check_var() {
  local name="$1"
  if [ -n "${!name:-}" ]; then
    echo "$name: set"
  else
    echo "$name: missing"
    status=1
  fi
}

for name in GITHUB_APP_ID GITHUB_WEBHOOK_SECRET LIVE_SMOKE_TUNNEL_URL; do
  check_var "$name"
done

if [ -n "${GITHUB_APP_PRIVATE_KEY:-}" ] || [ -n "${GITHUB_APP_PRIVATE_KEY_FILE:-}" ]; then
  echo "GITHUB_APP_PRIVATE_KEY(_FILE): set"
else
  echo "GITHUB_APP_PRIVATE_KEY(_FILE): missing"
  status=1
fi

if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
  echo "ANTHROPIC_API_KEY: set"
elif [ -n "${OPENAI_API_KEY:-}" ]; then
  echo "OPENAI_API_KEY: set"
else
  echo "ANTHROPIC_API_KEY|OPENAI_API_KEY: missing"
  status=1
fi

if [ "$status" -ne 0 ]; then
  echo "preflight failed: missing variables"
  exit 1
fi

if ! curl -fsS --max-time 10 -o /dev/null "${LIVE_SMOKE_TUNNEL_URL%/}/health" 2>/dev/null; then
  echo "tunnel: unreachable"
  exit 2
fi
echo "tunnel: reachable"

if [ "${LIVE_SMOKE_SKIP_MODEL_CALL:-0}" = "1" ]; then
  echo "model key: skipped"
else
  if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
    code=$(curl -s --max-time 30 -o /dev/null -w '%{http_code}' \
      -H "x-api-key: ${ANTHROPIC_API_KEY}" -H 'anthropic-version: 2023-06-01' \
      -H 'content-type: application/json' \
      -d '{"model":"'"${LIVE_SMOKE_MODEL:-claude-haiku-4-5}"'","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}' \
      https://api.anthropic.com/v1/messages)
  else
    code=$(curl -s --max-time 30 -o /dev/null -w '%{http_code}' \
      -H "authorization: Bearer ${OPENAI_API_KEY}" -H 'content-type: application/json' \
      -d '{"model":"'"${LIVE_SMOKE_MODEL:-gpt-4o-mini}"'","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}' \
      https://api.openai.com/v1/chat/completions)
  fi
  if [ "$code" != "200" ]; then
    echo "model key: rejected (HTTP $code)"
    exit 3
  fi
  echo "model key: ok"
fi

echo "preflight passed"
