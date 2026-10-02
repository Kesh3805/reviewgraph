#!/usr/bin/env bash
# Run any command inside the Linux engine build container (ADR-001).
#   engine/scripts/run.sh sqlx migrate run --source migrations
# The repository root is mounted at /repo; the cargo registry and target/ live in named
# volumes (REVIEWGRAPH_TARGET_VOLUME picks a per-lane target volume so parallel builds do not
# block each other). Set REVIEWGRAPH_DOCKER_NETWORK to join a compose network (e.g. reviewgraph_default)
# so services are reachable by name; otherwise host services are at host.docker.internal.
set -euo pipefail
ENGINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "$ENGINE_DIR/.." && pwd)"
IMAGE="${REVIEWGRAPH_BUILD_IMAGE:-reviewgraph-engine-dev:2}"

host_path() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "building $IMAGE ..." >&2
  MSYS_NO_PATHCONV=1 docker build -t "$IMAGE" \
    -f "$(host_path "$ENGINE_DIR/docker/dev.Dockerfile")" "$(host_path "$ENGINE_DIR/docker")" >&2
fi

TTY_FLAGS=(-i)
if [ -t 1 ] && [ -t 0 ]; then TTY_FLAGS=(-it); fi

NET_FLAGS=(--add-host host.docker.internal:host-gateway)
if [ -n "${REVIEWGRAPH_DOCKER_NETWORK:-}" ]; then
  NET_FLAGS+=(--network "$REVIEWGRAPH_DOCKER_NETWORK")
fi

ENV_FLAGS=()
for v in DATABASE_URL TEST_DATABASE_URL QDRANT_URL REDIS_URL RUST_LOG RUST_BACKTRACE \
         ANTHROPIC_API_KEY OPENAI_API_KEY VOYAGE_API_KEY OTEL_EXPORTER_OTLP_ENDPOINT \
         OTEL_EXPORTER_OTLP_HEADERS INSTA_UPDATE PROPTEST_CASES REVIEWGRAPH_FIXTURES \
         S3_ENDPOINT S3_BUCKET S3_ACCESS_KEY_ID S3_SECRET_ACCESS_KEY; do
  if [ -n "${!v:-}" ]; then ENV_FLAGS+=(-e "$v"); fi
done

WORKDIR="${REVIEWGRAPH_WORKDIR:-/repo/engine}"

exec env MSYS_NO_PATHCONV=1 docker run --rm "${TTY_FLAGS[@]}" "${NET_FLAGS[@]}" \
  -v "$(host_path "$REPO_DIR"):/repo" \
  -v rg-cargo-registry:/usr/local/cargo/registry \
  -v rg-cargo-git:/usr/local/cargo/git \
  -v "${REVIEWGRAPH_TARGET_VOLUME:-rg-engine-target}:/target" \
  -e CARGO_TARGET_DIR=/target \
  "${ENV_FLAGS[@]}" \
  -w "$WORKDIR" \
  "$IMAGE" "$@"
