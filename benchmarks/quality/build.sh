#!/usr/bin/env bash
# Materialise one benchmark case into a git repository (EVAL-001).
#
# Usage: benchmarks/quality/build.sh <case_dir> <out_dir>
#
# 1. Copies the shared template (templates/nest-app/), then the case's base/ overlay, and writes
#    review-config.yaml (if any) as .review/config.yaml.
# 2. Commits "base" with a fixed author, committer and date (2026-01-01T00:00:00Z).
# 3. Applies head.patch with `git apply --index` and commits "head" one hour later.
# 4. Writes `<base_sha> <head_sha>` to <out_dir>/.complete (last) and prints it.
#
# The fixed identity and dates make the SHAs identical on every machine. The repository is built
# in a temporary directory and renamed into place, so a failed build leaves nothing behind.
# Repository content is never executed.
set -euo pipefail

case_dir="${1:?usage: build.sh <case_dir> <out_dir>}"
out_dir="${2:?usage: build.sh <case_dir> <out_dir>}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
template="$here/templates/nest-app"
case_id="$(basename "$case_dir")"

export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null TZ=UTC LC_ALL=C
export GIT_AUTHOR_NAME="ReviewGraph Benchmarks" GIT_AUTHOR_EMAIL="benchmarks@reviewgraph.invalid"
export GIT_COMMITTER_NAME="ReviewGraph Benchmarks" GIT_COMMITTER_EMAIL="benchmarks@reviewgraph.invalid"

[ -f "$case_dir/case.yaml" ] || { echo "build.sh: $case_id: no case.yaml" >&2; exit 2; }
[ -f "$case_dir/head.patch" ] || { echo "build.sh: $case_id: no head.patch" >&2; exit 2; }

tmp="${out_dir}.tmp.$$"
rm -rf "$tmp"
mkdir -p "$tmp"
trap 'rm -rf "$tmp"' EXIT

git init -q --template= -b main "$tmp"
git -C "$tmp" config core.autocrlf false
git -C "$tmp" config core.fileMode false
git -C "$tmp" config core.symlinks false
git -C "$tmp" config commit.gpgsign false

cp -R "$template/." "$tmp/"
if [ -d "$case_dir/base" ]; then
  cp -R "$case_dir/base/." "$tmp/"
fi
if [ -f "$case_dir/review-config.yaml" ]; then
  mkdir -p "$tmp/.review"
  cp "$case_dir/review-config.yaml" "$tmp/.review/config.yaml"
fi

git -C "$tmp" add -A
GIT_AUTHOR_DATE="2026-01-01T00:00:00Z" GIT_COMMITTER_DATE="2026-01-01T00:00:00Z" \
  git -C "$tmp" commit -q --no-verify -m "base"
base_sha="$(git -C "$tmp" rev-parse HEAD)"

if ! git -C "$tmp" apply --index --whitespace=nowarn "$(cd "$case_dir" && pwd)/head.patch"; then
  echo "build.sh: $case_id: head.patch does not apply" >&2
  exit 1
fi
GIT_AUTHOR_DATE="2026-01-01T01:00:00Z" GIT_COMMITTER_DATE="2026-01-01T01:00:00Z" \
  git -C "$tmp" commit -q --no-verify --allow-empty -m "head"
head_sha="$(git -C "$tmp" rev-parse HEAD)"

rm -rf "$out_dir"
mkdir -p "$(dirname "$out_dir")"
mv "$tmp" "$out_dir"
trap - EXIT
printf '%s %s\n' "$base_sha" "$head_sha" > "$out_dir/.complete"
echo "$base_sha $head_sha"
