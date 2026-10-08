/**
 * Secret redaction for text that leaves the API: source excerpts served to the UI (API-011)
 * and audit metadata (SEC-008). For excerpts the engine already
 * redacts (API-013 runs `telemetry::redact`); the proxy re-applies the same kind of rules so an
 * excerpt is never shown unredacted even if the engine side regresses. Structure is kept:
 * `API_KEY = "abc"` becomes `API_KEY="<redacted>"`.
 */
export const REDACTED = '<redacted>';

/** Identifiers whose assigned value is treated as a secret. */
const SENSITIVE_NAME = String.raw`[A-Za-z0-9_.-]*(?:KEY|SECRET|TOKEN|PASSWORD|PASSWD|PWD|CREDENTIAL|PRIVATE|AUTH)[A-Za-z0-9_.-]*`;

/** `NAME = "value"`, `NAME: 'value'`, `NAME=value` (env files), case-insensitive on the name. */
const ASSIGNMENT = new RegExp(
  String.raw`\b(${SENSITIVE_NAME})(\s*(?::|=(?!=))\s*)(?:"[^"\n]*"|'[^'\n]*'|\x60[^\x60\n]*\x60|(?:Bearer|Basic|Token)\s+[^\s,;)}\]]+|[^\s,;)}\]]+)`,
  'gi',
);

/** Well-known token shapes, redacted wherever they appear. */
const TOKEN_PATTERNS: RegExp[] = [
  /\bAKIA[0-9A-Z]{16}\b/g, // AWS access key id
  /\bgh[pousr]_[A-Za-z0-9]{36,255}\b/g, // GitHub tokens
  /\bgithub_pat_[A-Za-z0-9_]{22,255}\b/g,
  /\bsk-[A-Za-z0-9_-]{20,}\b/g, // provider API keys
  /\bxox[abprs]-[A-Za-z0-9-]{10,}\b/g, // Slack tokens
  /\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/g, // JWTs
  /(Bearer\s+)[A-Za-z0-9._~+/-]{16,}=*/g,
];

const PRIVATE_KEY_BLOCK =
  /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)/g;

export function redactExcerpt(text: string): { text: string; redactions: number } {
  let redactions = 0;
  let out = text.replace(PRIVATE_KEY_BLOCK, () => {
    redactions++;
    return `-----BEGIN PRIVATE KEY-----${REDACTED}-----END PRIVATE KEY-----`;
  });
  out = out.replace(ASSIGNMENT, (_m, name: string, sep: string) => {
    redactions++;
    // Keep the separator style but normalize the value to a quoted placeholder.
    return `${name}${sep.includes(':') ? sep : '='}"${REDACTED}"`;
  });
  for (const pattern of TOKEN_PATTERNS) {
    out = out.replace(pattern, (m: string, prefix?: string) => {
      redactions++;
      return typeof prefix === 'string' && m.startsWith(prefix) ? `${prefix}${REDACTED}` : REDACTED;
    });
  }
  return { text: out, redactions };
}

/** Object keys whose string values are secrets (narrower than the assignment names). */
const SENSITIVE_KEY =
  /(secret|token|password|passwd|credential|api_?key|private_?key|authorization|cookie)/i;

/**
 * Redacts a JSON-like value: string leaves go through `redactExcerpt`, and the value of any key
 * with a sensitive name (`token`, `client_secret`, ...) is replaced as a whole.
 */
export function redactValue(value: unknown, depth = 0): unknown {
  if (depth > 20) return REDACTED;
  if (typeof value === 'string') return redactExcerpt(value).text;
  if (Array.isArray(value)) return value.map((v) => redactValue(v, depth + 1));
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [
        k,
        SENSITIVE_KEY.test(k) && typeof v === 'string' ? REDACTED : redactValue(v, depth + 1),
      ]),
    );
  }
  return value;
}
