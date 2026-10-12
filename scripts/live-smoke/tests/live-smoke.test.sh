#!/usr/bin/env bash
# Script tests for preflight.sh and check-report.sh. Run: bash scripts/live-smoke/tests/live-smoke.test.sh
set -u
here="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0

t() { # name, expected exit, actual exit
  if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (expected $2, got $3)"; fail=1; fi
}

# --- preflight ---
out="$(env -i PATH="$PATH" LIVE_SMOKE_ENV_FILE=/nonexistent GITHUB_WEBHOOK_SECRET=topsecretvalue bash "$here/preflight.sh" 2>&1)"
code=$?
t preflight_reports_missing_variables_exit 1 "$code"
case "$out" in *GITHUB_APP_ID:\ missing*) echo "ok   preflight_names_missing_variable" ;; *) echo "FAIL preflight_names_missing_variable"; fail=1 ;; esac
case "$out" in *topsecretvalue*) echo "FAIL preflight_never_prints_values"; fail=1 ;; *) echo "ok   preflight_never_prints_values" ;; esac

env -i PATH="$PATH" LIVE_SMOKE_ENV_FILE=/nonexistent GITHUB_APP_ID=1 GITHUB_WEBHOOK_SECRET=x \
  GITHUB_APP_PRIVATE_KEY=k ANTHROPIC_API_KEY=k LIVE_SMOKE_TUNNEL_URL=http://127.0.0.1:1 \
  LIVE_SMOKE_SKIP_MODEL_CALL=1 bash "$here/preflight.sh" >/dev/null 2>&1
t preflight_fails_on_unreachable_tunnel 2 $?

# --- check-report ---
repo="$tmp/repo"
mkdir -p "$repo/reports" && cd "$repo" || exit 1
git init -q . && git -c user.email=t@example.com -c user.name=t commit -q --allow-empty -m init
sha="$(git rev-parse HEAD)"
export LIVE_SMOKE_REPORT_DIR="$repo/reports"

bash "$here/check-report.sh" "$sha" >/dev/null 2>&1
t check_report_fails_when_report_absent 1 $?

printf 'Commit: %s\n- [x] done\n- [ ] not done\nSigned-off-by: A Person 2026-01-01\n' "$sha" > reports/r.md
bash "$here/check-report.sh" "$sha" >/dev/null 2>&1
t check_report_fails_when_unticked_box_present 1 $?

printf 'Commit: %s\n- [x] done\nSigned-off-by: A Person 2026-01-01\n' "$sha" > reports/r.md
bash "$here/check-report.sh" "$sha" >/dev/null 2>&1
t check_report_passes_for_signed_report 0 $?

exit $fail
