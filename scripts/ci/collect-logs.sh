#!/usr/bin/env bash
# Collects service-container logs and a Postgres slow-query excerpt into an artifact directory.
# Runs on the CI runner only (`if: failure()`); it is a no-op where docker is not installed.
#   collect-logs.sh [output-dir]
set -uo pipefail
out="${1:-ci-logs}"
mkdir -p "$out"
if ! command -v docker >/dev/null 2>&1; then
  echo "docker not available; no container logs to collect" | tee "$out/README.txt"
  exit 0
fi
docker ps -a --format '{{.ID}} {{.Names}} {{.Image}} {{.Status}}' > "$out/containers.txt" 2>&1 || true
while read -r id name _; do
  [ -n "$id" ] || continue
  docker logs --timestamps "$id" > "$out/${name//\//_}.log" 2>&1 || true
done < "$out/containers.txt"
for f in "$out"/*postgres*.log; do
  [ -f "$f" ] || continue
  grep -E 'duration: [0-9]+\.[0-9]+ ms' "$f" > "${f%.log}.slow-queries.txt" || true
done
echo "collected $(find "$out" -type f | wc -l) file(s) into $out"
