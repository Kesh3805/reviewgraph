/**
 * Defense-in-depth redaction of source excerpts served to the UI (API-011). The engine already
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
