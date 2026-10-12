/** Short commit SHA for tables. */
export function shortSha(sha: string | null | undefined): string {
  return sha ? sha.slice(0, 7) : '-';
}

/** Locale-independent timestamp (`2026-10-08 14:03 UTC`), stable across server and client. */
export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) return '-';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '-';
  return `${d.toISOString().slice(0, 16).replace('T', ' ')} UTC`;
}

/** Compact relative age (`42s`, `5m`, `3h`, `2d`). */
export function formatAge(iso: string, now = Date.now()): string {
  const seconds = Math.max(0, Math.floor((now - new Date(iso).getTime()) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

export function formatDuration(ms: number | null | undefined): string {
  if (ms == null) return '-';
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
}

export function formatPercent(fraction: number, digits = 0): string {
  return `${(fraction * 100).toFixed(digits)}%`;
}

export function formatNumber(n: number): string {
  return n.toLocaleString('en-US');
}

/** Micro-dollars (`cost_usd_micros`) to a dollar string. */
export function formatUsdMicros(micros: number | null | undefined): string {
  if (micros == null) return '-';
  const usd = micros / 1_000_000;
  return `$${usd < 1 ? usd.toFixed(4) : usd.toFixed(2)}`;
}
