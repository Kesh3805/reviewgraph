#!/usr/bin/env bash
# Waits for service endpoints and fails with the name of every service that did not come up.
#   wait-for-stack.sh [--timeout SECONDS] name=host:port [name=http://host:port/path ...]
# A host:port target is probed with a TCP connect, an http(s) URL with `curl -f`.
set -uo pipefail

timeout_s=180
if [ "${1:-}" = "--timeout" ]; then timeout_s="$2"; shift 2; fi
[ "$#" -gt 0 ] || { echo "usage: wait-for-stack.sh [--timeout SECONDS] name=target ..." >&2; exit 2; }

probe() {
  local target="$1"
  case "$target" in
    http://* | https://*) curl -fsS --max-time 3 -o /dev/null "$target" 2>/dev/null ;;
    *)
      local host="${target%:*}" port="${target##*:}"
      (exec 3<>"/dev/tcp/${host}/${port}") 2>/dev/null
      ;;
  esac
}

start=$SECONDS
pending=("$@")
while [ "${#pending[@]}" -gt 0 ]; do
  next=()
  for spec in "${pending[@]}"; do
    if probe "${spec#*=}"; then
      echo "ready: ${spec%%=*} after $((SECONDS - start))s"
    else
      next+=("$spec")
    fi
  done
  pending=("${next[@]+"${next[@]}"}")
  [ "${#pending[@]}" -eq 0 ] && break
  if [ $((SECONDS - start)) -ge "$timeout_s" ]; then
    for spec in "${pending[@]}"; do echo "TIMEOUT: service '${spec%%=*}' (${spec#*=}) not ready after ${timeout_s}s" >&2; done
    exit 1
  fi
  sleep 1
done
echo "stack ready in $((SECONDS - start))s"
