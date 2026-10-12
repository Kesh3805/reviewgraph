#!/usr/bin/env bash
# Usage: image-smoke.sh <image> <name>
# Starts the image with a read-only rootfs and dropped capabilities (CI runner only).
set -euo pipefail
img="$1"; name="$2"
hardening=(--read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true)

if [ "$name" = "engine" ]; then
  timeout 30 docker run --rm "${hardening[@]}" "$img" review --version
  timeout 30 docker run --rm "${hardening[@]}" "$img" review-worker --help >/dev/null
  echo "OK: engine smoke"
  exit 0
fi

# Dummy dev-only configuration so the process can start without any backing service.
api_env=(
  -e NODE_ENV=production
  -e GITHUB_ENABLED=false
  -e DATABASE_URL=postgres://smoke:smoke@127.0.0.1:1/smoke
  -e REDIS_URL=redis://127.0.0.1:1
  -e ENGINE_INTERNAL_URL=http://127.0.0.1:3100
  -e SERVICE_JWT_SECRET=smoke-only-service-jwt-secret-0123456789abcdef
  -e SESSION_JWT_SECRET=smoke-only-session-jwt-secret-0123456789abcdef
  -e TOKEN_CACHE_KEY=MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=
  -e WEB_ORIGIN=http://127.0.0.1:3001
  -e QUEUE_CONSUMERS_ENABLED=false
  -e RG_OTEL_ENABLED=false
)
case "$name" in
  api) port=3000; path=/health; extra=("${api_env[@]}") ;;
  web) port=3001; path=/login; extra=() ;;
  *) echo "unknown image name: $name" >&2; exit 2 ;;
esac

cid=$(docker run -d "${hardening[@]}" -p "127.0.0.1::${port}" "${extra[@]}" "$img")
cleanup() { docker rm -f "$cid" >/dev/null 2>&1 || true; }
trap cleanup EXIT
host_port=$(docker port "$cid" "${port}/tcp" | head -n1 | sed 's/.*://')
deadline=$((SECONDS + 30))
while [ "$SECONDS" -lt "$deadline" ]; do
  if [ "$(docker inspect --format '{{.State.Running}}' "$cid")" != "true" ]; then
    docker logs "$cid" || true
    echo "FAIL: $name exited before answering"; exit 1
  fi
  code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${host_port}${path}" || true)
  if [ "$code" != "000" ] && [ "$code" -lt 500 ]; then
    echo "OK: $name smoke (GET $path -> $code)"; exit 0
  fi
  sleep 1
done
docker logs "$cid" || true
echo "FAIL: $name did not answer GET $path within 30s"; exit 1
