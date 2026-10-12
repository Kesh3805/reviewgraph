#!/usr/bin/env bash
# Deterministic fixture repository builder (FND-008).
#
# Usage:
#   fixtures/build.sh <name>|--all [--out DIR] [--src DIR] [--update-expected]
#   fixtures/build.sh --pr <scenario> [--update-expected]   (pull-request scenarios, DIFF-007)
#
# Fixture format. fixtures/repositories/<name>/steps/ holds directories 001-<slug>,
# 002-<slug>, ... processed in LC_ALL=C lexical order. Each step may contain:
#   _commit.txt  (required) the commit message.
#   _ops.txt     (optional) one operation per line, run BEFORE the overlay is copied:
#                  rm <path>        git rm -q
#                  mv <from> <to>   git mv
#                  branch <name>    git switch -c
#                  switch <name>    git switch
#   anything else is an overlay, copied over the working tree at the same relative path.
# Files starting with `_` at the step root are control files and are never copied.
# Symlinks and executable files are forbidden and rejected.
#
# Output: <out>/<name>/ (default fixtures/.build/repos/<name>, deleted and recreated), one
# commit per step, tagged step-NNN, plus <out>/<name>.shas with one `step-NNN <sha>` line per
# step. Commit dates are 2026-01-01T00:00:00Z + step-index hours, so SHAs are reproducible on
# every machine. --update-expected rewrites <name>/EXPECTED_SHAS from the built SHAs.
#
# Concurrency: builds of different fixtures are independent; building the same fixture
# concurrently is unsafe. flock fixtures/.build/.lock is taken when flock is available.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$SCRIPT_DIR/repositories"
OUT="$SCRIPT_DIR/.build/repos"
EPOCH=1767225600 # 2026-01-01T00:00:00Z

target=""
update_expected=0
pr=0
while [ $# -gt 0 ]; do
  case "$1" in
    --all) target="--all" ;;
    --out) OUT="$2"; shift ;;
    --src) SRC="$2"; shift ;;
    --update-expected) update_expected=1 ;;
    --pr) pr=1 ;;
    -*) echo "build.sh: unknown option $1" >&2; exit 2 ;;
    *) target="$1" ;;
  esac
  shift
done
[ -n "$target" ] || { echo "usage: build.sh <name>|--all [--out DIR] [--src DIR] [--update-expected]" >&2; exit 2; }

date -u -d "@$EPOCH" +%Y-%m-%dT%H:%M:%SZ >/dev/null 2>&1 \
  || { echo "build.sh: GNU 'date -d' is required (Git Bash and Linux provide it)" >&2; exit 2; }

export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null TZ=UTC LC_ALL=C
export XDG_CONFIG_HOME="$SCRIPT_DIR/.build/.no-xdg"

mkdir -p "$SCRIPT_DIR/.build"
if command -v flock >/dev/null 2>&1; then
  exec 9>"$SCRIPT_DIR/.build/.lock"
  flock 9
fi

# Reject symlinks and executables in a step directory. Modes come from the git index when the
# fixture is tracked (reliable on every OS); symlinks are detected on disk.
check_step_files() {
  local step="$1"
  if [ -n "$(find "$step" -type l -print -quit)" ]; then
    echo "build.sh: symlink found in $step" >&2; return 1
  fi
  local modes
  modes="$(git -c safe.directory='*' ls-files -s -- "$step" 2>/dev/null | cut -d' ' -f1 | sort -u || true)"
  if printf '%s\n' "$modes" | grep -qE '^(100755|120000)$'; then
    echo "build.sh: executable file or symlink tracked in $step" >&2; return 1
  fi
  return 0
}

build_one() {
  local name="$1"
  local fixture="$SRC/$name"
  local steps="$fixture/steps"
  local dst="$OUT/$name"
  [ -d "$steps" ] || { echo "build.sh: no such fixture: $name ($steps)" >&2; return 1; }

  rm -rf "$dst"
  mkdir -p "$OUT"
  local ok=0
  trap '[ "$ok" = 1 ] || rm -rf "$dst"' RETURN

  git init -q --template= -b main "$dst"
  git -C "$dst" config core.autocrlf false
  git -C "$dst" config core.fileMode false
  git -C "$dst" config core.symlinks false
  git -C "$dst" config commit.gpgsign false
  git -C "$dst" config user.name "ReviewGraph Fixtures"
  git -C "$dst" config user.email "fixtures@reviewgraph.invalid"

  : > "$OUT/$name.shas"
  local i=0 step slug num
  while IFS= read -r step; do
    i=$((i + 1))
    slug="$(basename "$step")"
    num="${slug%%-*}"
    if [ ! -f "$step/_commit.txt" ]; then
      echo "build.sh: $name/$slug: missing _commit.txt" >&2; return 1
    fi
    check_step_files "$step" || { echo "build.sh: $name/$slug rejected" >&2; return 1; }

    if [ -f "$step/_ops.txt" ]; then
      local op a b rest
      while IFS= read -r line || [ -n "$line" ]; do
        line="${line%$'\r'}"
        [ -n "$line" ] || continue
        read -r op a b rest <<<"$line"
        case "$op" in
          rm) git -C "$dst" rm -q -- "$a" ;;
          mv) mkdir -p "$dst/$(dirname "$b")"; git -C "$dst" mv -- "$a" "$b" ;;
          branch) git -C "$dst" switch -q -c "$a" ;;
          switch) git -C "$dst" switch -q "$a" ;;
          *) echo "build.sh: $name/$slug: unknown op '$op'" >&2; return 1 ;;
        esac
      done < "$step/_ops.txt"
    fi

    local rel
    while IFS= read -r rel; do
      rel="${rel#./}"
      case "$rel" in _*/*) ;; _*) continue ;; esac
      mkdir -p "$dst/$(dirname "$rel")"
      cp "$step/$rel" "$dst/$rel"
    done < <(cd "$step" && find . -type f | sort)

    git -C "$dst" add -A
    local d
    d="$(date -u -d "@$((EPOCH + i * 3600))" +%Y-%m-%dT%H:%M:%SZ)"
    GIT_AUTHOR_DATE="$d" GIT_COMMITTER_DATE="$d" \
      git -C "$dst" commit -q --no-verify --allow-empty -F "$step/_commit.txt"
    git -C "$dst" tag "step-$num"
    printf 'step-%s %s\n' "$num" "$(git -C "$dst" rev-parse HEAD)" >> "$OUT/$name.shas"
  done < <(find "$steps" -mindepth 1 -maxdepth 1 -type d | sort)

  if [ "$update_expected" = 1 ]; then
    cp "$OUT/$name.shas" "$fixture/EXPECTED_SHAS"
  fi
  ok=1
  echo "built $name ($i steps)"
}

# Pull-request scenarios (DIFF-007): fixtures/pull-requests/<name>/{base/, patch.diff}.
# Builds a 2-commit repository (base, then base + patch) at <out>/../prs/<name> with the same
# fixed identity/dates, writes <name>.shas and the git references `git diff --histogram -U3` and
# `--name-status -M50%` next to it (--update-expected copies them into <name>/expected/).
# The Rust tests build the same trees in-process (diff_engine::testkit::Scenario); this mode is
# for inspecting a scenario with git and for regenerating the references.
build_pr() {
  local name="$1"
  local scen="$SCRIPT_DIR/pull-requests/$name"
  local out="$SCRIPT_DIR/.build/prs"
  local dst="$out/$name"
  [ -d "$scen/base" ] && [ -f "$scen/patch.diff" ] \
    || { echo "build.sh: no such pull-request scenario: $name" >&2; return 1; }
  rm -rf "$dst"
  mkdir -p "$out"
  git init -q --template= -b main "$dst"
  git -C "$dst" config core.autocrlf false
  git -C "$dst" config commit.gpgsign false
  git -C "$dst" config user.name fixture
  git -C "$dst" config user.email fixture@example.com
  cp -R "$scen/base/." "$dst/"
  git -C "$dst" add -A
  local d
  d="$(date -u -d "@$EPOCH" +%Y-%m-%dT%H:%M:%SZ)"
  GIT_AUTHOR_DATE="$d" GIT_COMMITTER_DATE="$d" git -C "$dst" commit -q --no-verify -m base
  git -C "$dst" apply --whitespace=nowarn "$scen/patch.diff"
  git -C "$dst" add -A
  GIT_AUTHOR_DATE="$d" GIT_COMMITTER_DATE="$d" git -C "$dst" commit -q --no-verify -m head
  printf 'base %s\nhead %s\n' "$(git -C "$dst" rev-parse HEAD~1)" "$(git -C "$dst" rev-parse HEAD)" \
    > "$out/$name.shas"
  git -C "$dst" diff --histogram -U3 --no-color HEAD~1 HEAD > "$out/$name.git-diff-u3.patch"
  git -C "$dst" diff --name-status -M50% HEAD~1 HEAD > "$out/$name.name-status.txt"
  if [ "$update_expected" = 1 ]; then
    mkdir -p "$scen/expected"
    cp "$out/$name.git-diff-u3.patch" "$scen/expected/git-diff-u3.patch"
    cp "$out/$name.name-status.txt" "$scen/expected/name-status.txt"
  fi
  echo "built pull-request $name"
}

if [ "$pr" = 1 ]; then
  build_pr "$target"
elif [ "$target" = "--all" ]; then
  names="$(find "$SRC" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort)"
  [ -n "$names" ] || { echo "build.sh: no fixtures under $SRC" >&2; exit 1; }
  for n in $names; do build_one "$n"; done
else
  build_one "$target"
fi
