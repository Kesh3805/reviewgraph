#!/usr/bin/env bash
# Release gate (E2E-002): fails unless a signed live-smoke report exists for <commit>.
#
# A report matches when its `Commit:` line is <commit>, or an ancestor of <commit> that is at
# most 30 days older with no changes under engine/ in between. Reports live in
# docs/operations/live-smoke-reports/ (override with LIVE_SMOKE_REPORT_DIR).
# A report passes when it has no unticked `- [ ]` box and a non-empty `Signed-off-by:` line.
set -u

commit="${1:-}"
if [ -z "$commit" ]; then
  echo "usage: check-report.sh <commit>" >&2
  exit 2
fi
dir="${LIVE_SMOKE_REPORT_DIR:-docs/operations/live-smoke-reports}"
full=$(git rev-parse --verify --quiet "${commit}^{commit}") || { echo "unknown commit: $commit" >&2; exit 2; }

matches=""
for report in "$dir"/*.md; do
  [ -f "$report" ] || continue
  case "$(basename "$report")" in TEMPLATE.md) continue ;; esac
  tested=$(sed -n 's/^Commit:[[:space:]]*\([0-9a-fA-F]\{7,40\}\).*/\1/p' "$report" | head -n 1)
  [ -n "$tested" ] || continue
  tested_full=$(git rev-parse --verify --quiet "${tested}^{commit}") || continue
  if [ "$tested_full" = "$full" ]; then
    matches="$matches $report"
    continue
  fi
  if git merge-base --is-ancestor "$tested_full" "$full" 2>/dev/null \
    && git diff --quiet "$tested_full" "$full" -- engine; then
    age=$(( $(git show -s --format=%ct "$full") - $(git show -s --format=%ct "$tested_full") ))
    if [ "$age" -le $((30 * 86400)) ]; then
      matches="$matches $report"
    fi
  fi
done

if [ -z "$matches" ]; then
  echo "no live-smoke report found for $commit in $dir"
  exit 1
fi

for report in $matches; do
  if grep -qE '^[[:space:]]*- \[ \]' "$report"; then
    echo "$report has unticked boxes"
    continue
  fi
  signer=$(sed -n 's/^Signed-off-by:[[:space:]]*//p' "$report" | head -n 1)
  if [ -z "$signer" ]; then
    echo "$report is not signed"
    continue
  fi
  echo "live-smoke report OK: $report"
  exit 0
done
exit 1
