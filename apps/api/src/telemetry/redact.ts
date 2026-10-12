/**
 * Redaction patterns shared (by corpus) with the Rust `telemetry::redact` module.
 * Replacement format: `«redacted:<pattern>»`. Redacting redacted text is a no-op.
 */
interface Pattern {
  name: string;
  regex: RegExp;
}

const tag = (name: string): string => `«redacted:${name}»`;

export const PATTERNS: readonly Pattern[] = [
  {
    name: 'private_key',
    regex: /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g,
  },
  { name: 'aws_access_key', regex: /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g },
  {
    name: 'github_token',
    regex: /\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b/g,
  },
  { name: 'api_key', regex: /\bsk-(?:ant-)?[A-Za-z0-9_-]{20,}/g },
  { name: 'jwt', regex: /\beyJ[A-Za-z0-9_-]{5,}\.eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}/g },
  {
    name: 'auth_header',
    regex: /(?:authorization\s*[:=]\s*)?\b(?:Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{6,}/gi,
  },
  { name: 'hub_signature', regex: /sha256=[0-9a-f]{64}\b/gi },
  {
    name: 'url_userinfo',
    regex: /(?<=\b[a-z][a-z0-9+.-]*:\/\/)[^\s/@:]+:[^\s/@]+(?=@)/gi,
  },
  {
    name: 'env_assignment',
    regex: /\b[A-Z0-9_]*(?:KEY|SECRET|TOKEN|PASSWORD|PASSWD|PRIVATE)[A-Z0-9_]*\s*[=:]\s*\S+/g,
  },
];

export const SENSITIVE_KEYS = [
  'authorization',
  'cookie',
  'password',
  'token',
  'secret',
  'private_key',
];
export const MAX_FIELD_BYTES = 4096;
const TRUNCATED = '…[truncated]';
const MAX_DEPTH = 6;

export function redactString(input: string): string {
  let out = input;
  for (const p of PATTERNS) out = out.replace(p.regex, tag(p.name));
  if (Buffer.byteLength(out) > MAX_FIELD_BYTES) {
    out = out.slice(0, MAX_FIELD_BYTES) + TRUNCATED;
  }
  return out;
}

function sensitiveKey(key: string): boolean {
  const k = key.toLowerCase();
  return SENSITIVE_KEYS.some((s) => k.includes(s));
}

/** Deep, depth-limited, circular-safe object redaction. */
export function redactObject<T>(value: T, depth = 0, seen = new WeakSet<object>()): T {
  if (typeof value === 'string') return redactString(value) as T;
  if (value === null || typeof value !== 'object') return value;
  if (depth >= MAX_DEPTH) return '«redacted:depth»' as T;
  if (seen.has(value)) return '[circular]' as T;
  seen.add(value);
  if (Array.isArray(value)) return value.map((v) => redactObject(v, depth + 1, seen)) as T;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
    out[k] = sensitiveKey(k) ? tag('sensitive_key') : redactObject(v, depth + 1, seen);
  }
  return out as T;
}

/** Scrubs string attributes of a finished span before export. */
export function redactAttributes(attributes: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(attributes)) {
    out[k] = sensitiveKey(k) ? tag('sensitive_key') : typeof v === 'string' ? redactString(v) : v;
  }
  return out;
}
