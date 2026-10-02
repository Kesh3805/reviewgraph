#!/usr/bin/env bash
# Run cargo inside the Linux engine build container (ADR-001).
#   engine/scripts/cargo.sh test --workspace
#   engine/scripts/cargo.sh run -p review-cli -- init --repository /repo/fixtures/...
# The repository root is mounted at /repo; target/ and the cargo registry live in
# named volumes so rebuilds are incremental.
set -euo pipefail
ENGINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "$ENGINE_DIR/.." && pwd)"
IMAGE="${REVIEWGRAPH_BUILD_IMAGE:-reviewgraph-engine-dev:1}"

host_path() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "building $IMAGE ..." >&2
  MSYS_NO_PATHCONV=1 docker build -t "$IMAGE" -f "$(host_path "$ENGINE_DIR/docker/dev.Dockerfile")" "$(host_path "$ENGINE_DIR/docker")" >&2
fi

TTY_FLAGS=(-i)
if [ -t 1 ] && [ -t 0 ]; then TTY_FLAGS=(-it); fi

ENV_FLAGS=()
for v in DATABASE_URL TEST_DATABASE_URL QDRANT_URL REDIS_URL RUST_LOG RUST_BACKTRACE \
         ANTHROPIC_API_KEY OPENAI_API_KEY VOYAGE_API_KEY OTEL_EXPORTER_OTLP_ENDPOINT \
         OTEL_EXPORTER_OTLP_HEADERS INSTA_UPDATE PROPTEST_CASES REVIEWGRAPH_FIXTURES; do
  if [ -n "${!v:-}" ]; then ENV_FLAGS+=(-e "$v"); fi
done

exec env MSYS_NO_PATHCONV=1 docker run --rm "${TTY_FLAGS[@]}" \
  --add-host host.docker.internal:host-gateway \
  -v "$(host_path "$REPO_DIR"):/repo" \
  -v rg-cargo-registry:/usr/local/cargo/registry \
  -v rg-cargo-git:/usr/local/cargo/git \
  -v rg-engine-target:/target \
  -e CARGO_TARGET_DIR=/target \
  "${ENV_FLAGS[@]}" \
  -w /repo/engine \
  "$IMAGE" cargo "$@"
