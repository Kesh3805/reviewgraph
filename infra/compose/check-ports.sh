#!/usr/bin/env bash
# Asserts every published port in the compose files binds to 127.0.0.1 only.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
status=0
for f in docker-compose.yml docker-compose.test.yml; do
  json="$(docker compose -f "$f" config --format json)"
  bad="$(printf '%s' "$json" | node -e '
    let s = ""; process.stdin.on("data", d => s += d).on("end", () => {
      const cfg = JSON.parse(s); const bad = [];
      for (const [name, svc] of Object.entries(cfg.services || {}))
        for (const p of svc.ports || [])
          if (p.host_ip !== "127.0.0.1") bad.push(`${name}:${p.published}`);
      process.stdout.write(bad.join(" "));
    });')"
  if [ -n "$bad" ]; then echo "$f: non-loopback ports: $bad" >&2; status=1; fi
done
[ $status -eq 0 ] && echo "all published ports are loopback-only"
exit $status
