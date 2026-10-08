#!/usr/bin/env bash
# Builds every fixture and compares the resulting SHAs with the committed EXPECTED_SHAS.
#   fixtures/check.sh                run the check
#   fixtures/check.sh --self-test    run the named self-test cases
# Cross-environment check (host_and_container_same_shas): run this script on the host and
# again through `engine/scripts/run.sh bash /repo/fixtures/check.sh`; both must exit 0.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="$SCRIPT_DIR/build.sh"

check() {
  bash "$BUILD" --all >/dev/null
  local failed=0 shas name
  for shas in "$SCRIPT_DIR"/.build/repos/*.shas; do
    name="$(basename "$shas" .shas)"
    if ! diff -u "$SCRIPT_DIR/repositories/$name/EXPECTED_SHAS" "$shas"; then
      echo "check.sh: $name SHAs differ from EXPECTED_SHAS" >&2
      failed=1
    fi
  done
  if [ "$failed" = 1 ]; then
    echo "if intentional, run fixtures/build.sh --all --update-expected" >&2
    return 1
  fi
  echo "fixtures ok"
}

self_test() {
  local pass=0
  tmp="$(mktemp -d)" # global: read by the EXIT trap
  trap 'rm -rf "$tmp"' EXIT

  build_twice_same_shas() {
    bash "$BUILD" --all --out "$tmp/a" >/dev/null
    bash "$BUILD" --all --out "$tmp/b" >/dev/null
    local f
    for f in "$tmp"/a/*.shas; do cmp -s "$f" "$tmp/b/$(basename "$f")" || return 1; done
  }

  rejects_symlink_in_step() {
    local src="$tmp/sym/x/steps/001-a"
    mkdir -p "$src"
    echo msg > "$src/_commit.txt"
    echo hi > "$src/real.txt"
    if ! MSYS=winsymlinks:nativestrict ln -s real.txt "$src/link.txt" 2>/dev/null || [ ! -L "$src/link.txt" ]; then
      echo "  (skipped: cannot create symlinks here)"; return 0
    fi
    if bash "$BUILD" x --src "$tmp/sym" --out "$tmp/sym-out" >/dev/null 2>&1; then return 1; fi
    [ ! -d "$tmp/sym-out/x" ]
  }

  rejects_missing_commit_message() {
    local src="$tmp/nomsg/x/steps/001-a"
    mkdir -p "$src"
    echo hi > "$src/file.txt"
    if bash "$BUILD" x --src "$tmp/nomsg" --out "$tmp/nomsg-out" >/dev/null 2>&1; then return 1; fi
    [ ! -d "$tmp/nomsg-out/x" ]
  }

  rename_move_history_has_git_detectable_renames() {
    bash "$BUILD" rename-move --out "$tmp/rm" >/dev/null
    git -C "$tmp/rm/rename-move" log --follow -M --format=%D -- src/b/orders.ts | grep -q 'tag: step-001'
  }

  local t
  for t in build_twice_same_shas rejects_symlink_in_step rejects_missing_commit_message \
           rename_move_history_has_git_detectable_renames; do
    if "$t"; then echo "ok   $t"; pass=$((pass + 1)); else echo "FAIL $t" >&2; return 1; fi
  done
  echo "$pass self-tests passed"
}

case "${1:-}" in
  --self-test) self_test ;;
  "") check ;;
  *) echo "usage: check.sh [--self-test]" >&2; exit 2 ;;
esac
