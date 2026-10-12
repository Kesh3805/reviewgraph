#!/usr/bin/env bash
# Usage: image-hardening-check.sh <image> <name>
# Asserts (CI runner only): pinned bases, non-root numeric user, size within limit, no secret
# patterns in image history, and the three engine binaries.
set -euo pipefail
img="$1"; name="$2"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dockerfile="$root/infra/docker/${name}.Dockerfile"
case "$name" in engine) limit=400 ;; api) limit=300 ;; web) limit=250 ;; *) limit=400 ;; esac
limit="${RG_IMAGE_LIMIT_MB:-$limit}"

# base_images_pinned_not_latest: every external FROM carries an explicit non-latest tag or digest.
while read -r ref; do
  case "$ref" in
    *@sha256:*) ;;
    *:latest | *:) echo "FAIL: base image '$ref' is not pinned"; exit 1 ;;
    *:* | *'${'*) ;;
    scratch | build | runtime | deps) ;;
    *) echo "FAIL: base image '$ref' has no tag"; exit 1 ;;
  esac
done < <(awk 'toupper($1)=="FROM" { for (i = 2; i <= NF; i++) if ($i !~ /^--/) { print $i; break } }' "$dockerfile")

# images_run_as_non_root
user="$(docker inspect --format '{{.Config.User}}' "$img")"
if [ -z "$user" ] || [ "$user" = "root" ] || [ "${user%%:*}" = "0" ]; then
  echo "FAIL: $img runs as root (User='$user')"; exit 1
fi
case "${user%%:*}" in *[!0-9]*) echo "FAIL: $img user '$user' is not numeric"; exit 1 ;; esac

# image_size_within_limits
size_mb=$(( $(docker image inspect --format '{{.Size}}' "$img") / 1024 / 1024 ))
if [ "$size_mb" -gt "$limit" ]; then
  echo "FAIL: $img is ${size_mb}MB, limit ${limit}MB"; exit 1
fi

# no_secret_patterns_in_image_history
if docker history --no-trunc "$img" | grep -Eiq '(api[_-]?key|secret|password|BEGIN [A-Z ]*PRIVATE KEY)='; then
  echo "FAIL: secret-like pattern in image history"; exit 1
fi

# engine_image_contains_three_binaries
if [ "$name" = "engine" ]; then
  docker run --rm --entrypoint sh "$img" -c 'command -v review && command -v review-worker && command -v review-engine' >/dev/null \
    || { echo "FAIL: engine image is missing one of review, review-worker, review-engine"; exit 1; }
fi

echo "OK: $img user=$user size=${size_mb}MB limit=${limit}MB"
{
  echo "| ${name} | ${user} | ${size_mb} MB (limit ${limit}) |"
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
