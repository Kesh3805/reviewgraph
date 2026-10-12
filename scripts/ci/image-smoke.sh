#!/usr/bin/env bash
# Usage: image-smoke.sh <image> <name>
# Starts the image with a read-only rootfs and dropped capabilities.
set -euo pipefail
img="$1"; name="$2"
run=(docker run --rm --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true)
if [ "$name" = "engine" ]; then
  timeout 30 "${run[@]}" "$img" review-cli --version
  timeout 30 "${run[@]}" "$img" review-worker --help
else
  cid=$(docker run -d --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true "$img")
  sleep 10
  if [ "$(docker inspect --format '{{.State.Running}}' "$cid")" != "true" ]; then
    docker logs "$cid"; docker rm -f "$cid" >/dev/null; echo "FAIL: $name exited"; exit 1
  fi
  docker rm -f "$cid" >/dev/null
fi
echo "OK: $name smoke"
