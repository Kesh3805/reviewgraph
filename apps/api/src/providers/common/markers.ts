const FINDING_MARKER = /<!-- reviewgraph:finding=([A-Za-z0-9._:-]+) run=([A-Za-z0-9._:-]+) -->/;

/** The finding key (fingerprint) and run id of an inline comment's hidden marker. */
export function parseFindingMarker(body: string): { finding: string; run: string } | null {
  const m = FINDING_MARKER.exec(body);
  return m?.[1] && m[2] ? { finding: m[1], run: m[2] } : null;
}

/** True when `body` carries `marker` as a whole token (e.g. `reviewgraph:run=<id>`). */
export function hasMarker(body: string | null | undefined, marker: string): boolean {
  if (!body) return false;
  let from = 0;
  for (;;) {
    const at = body.indexOf(marker, from);
    if (at < 0) return false;
    const next = body.charAt(at + marker.length);
    if (next === '' || next === ' ' || next === '-' || next === '\n') return true;
    from = at + 1;
  }
}

/** The run id of a `reviewgraph:run=<id>` publish marker, or null for any other marker. */
export function runIdOfMarker(marker: string): string | null {
  const m = /^reviewgraph:run=([A-Za-z0-9._:-]+)$/.exec(marker);
  return m?.[1] ?? null;
}
