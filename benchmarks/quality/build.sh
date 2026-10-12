#!/usr/bin/env bash
# Materialise a benchmark case into a git repo with a deterministic base and head commit.
# Usage: build.sh <case_dir> <out_dir>   (prints "base_sha head_sha")
set -euo pipefail

case_dir="${1:?case dir}"
out_dir="${2:?out dir}"
[ -f "$case_dir/case.yaml" ] || { echo "no case.yaml in $case_dir" >&2; exit 1; }
[ -f "$case_dir/head.patch" ] || { echo "no head.patch in $case_dir" >&2; exit 1; }

tmp="$(mktemp -d "${out_dir}.tmp.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

export GIT_AUTHOR_NAME="eval" GIT_AUTHOR_EMAIL="eval@example.invalid"
export GIT_COMMITTER_NAME="eval" GIT_COMMITTER_EMAIL="eval@example.invalid"
export GIT_AUTHOR_DATE="2026-01-01T00:00:00Z" GIT_COMMITTER_DATE="2026-01-01T00:00:00Z"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null

git init -q -b main "$tmp"
if [ -d "$case_dir/base" ]; then cp -R "$case_dir/base/." "$tmp/"; fi
if [ -f "$case_dir/review-config.yaml" ]; then
  mkdir -p "$tmp/.review"
  cp "$case_dir/review-config.yaml" "$tmp/.review/config.yaml"
fi
git -C "$tmp" add -A
git -C "$tmp" commit -q -m base
base_sha="$(git -C "$tmp" rev-parse HEAD)"

git -C "$tmp" apply --index "$(cd "$case_dir" && pwd)/head.patch"
git -C "$tmp" commit -q -m head
head_sha="$(git -C "$tmp" rev-parse HEAD)"

touch "$tmp/.git/.complete"
rm -rf "$out_dir"
mv "$tmp" "$out_dir"
trap - EXIT
echo "$base_sha $head_sha"
