#!/usr/bin/env bash
# Usage: mark-done.sh TASK-ID...  — flips "Status: ☐" to "Status: ☑" in the P30-P37 task doc.
f="$(dirname "$0")/../../docs/planning/tasks/P30-P37-ui-ops-quality.md"
for id in "$@"; do
  ln=$(grep -n "^### $id " "$f" | cut -d: -f1)
  [ -n "$ln" ] && sed -i "$((ln+1))s/☐/☑/" "$f"
done
