#!/usr/bin/env bash
# Usage: image-hardening-check.sh <image> <name>
# Asserts: non-root numeric user, size within limit, no secret patterns in image history.
set -euo pipefail
img="$1"; name="$2"
case "$name" in engine) limit=400 ;; api) limit=300 ;; web) limit=250 ;; *) limit=400 ;; esac

user="$(docker inspect --format '{{.Config.User}}' "$img")"
if [ -z "$user" ] || [ "$user" = "root" ] || [ "${user%%:*}" = "0" ]; then
  echo "FAIL: $img runs as root (User='$user')"; exit 1
fi

size_mb=$(( $(docker image inspect --format '{{.Size}}' "$img") / 1024 / 1024 ))
if [ "$size_mb" -gt "$limit" ]; then
  echo "FAIL: $img is ${size_mb}MB, limit ${limit}MB"; exit 1
fi

if docker history --no-trunc "$img" | grep -Eiq '(api[_-]?key|secret|password|BEGIN [A-Z ]*PRIVATE KEY)='; then
  echo "FAIL: secret-like pattern in image history"; exit 1
fi
echo "OK: $img user=$user size=${size_mb}MB"
