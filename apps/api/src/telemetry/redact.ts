/**
 * Redaction patterns shared (by corpus) with the Rust `telemetry::redact` module.
 * Replacement format: `«redacted:<pattern>»`. Redacting redacted text is a no-op.
 */
import type { LogRecordProcessor, SdkLogRecord } from '@opentelemetry/sdk-logs';
import type { ReadableSpan, SpanProcessor } from '@opentelemetry/sdk-trace-base';

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
  if (value instanceof Error) {
    // Error fields are not enumerable; keep the shape pino's `err` serializer produces.
    return {
      type: value.name,
      message: redactString(value.message),
      stack: value.stack === undefined ? undefined : redactString(value.stack),
    } as T;
  }
  if (!Array.isArray(value)) {
    const proto = Object.getPrototypeOf(value) as unknown;
    // Dates, Buffers, Maps and class instances keep their own serialization.
    if (proto !== Object.prototype && proto !== null) return value;
  }
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

/** Attribute keys no span may carry: source, diff and prompt content never go on spans. */
export const FORBIDDEN_SPAN_ATTRIBUTES: readonly string[] = ['prompt', 'source', 'diff', 'body'];

/**
 * Scrubs finished spans before any exporter sees them (OBS-006). Register it before the
 * exporting processor; it mutates the attribute objects in place.
 */
export class RedactingSpanProcessor implements SpanProcessor {
  onStart(): void {}

  onEnd(span: ReadableSpan): void {
    const attributes = span.attributes as Record<string, unknown>;
    for (const key of Object.keys(attributes)) {
      if (FORBIDDEN_SPAN_ATTRIBUTES.includes(key)) delete attributes[key];
    }
    Object.assign(attributes, redactAttributes(attributes));
    for (const event of span.events) {
      if (event.attributes) Object.assign(event.attributes, redactAttributes(event.attributes));
    }
  }

  forceFlush(): Promise<void> {
    return Promise.resolve();
  }

  shutdown(): Promise<void> {
    return Promise.resolve();
  }
}

/** Redacts log record bodies and string attributes before the batch processor exports them. */
export class RedactingLogRecordProcessor implements LogRecordProcessor {
  onEmit(record: SdkLogRecord): void {
    if (typeof record.body === 'string') record.setBody(redactString(record.body));
    for (const [key, value] of Object.entries(redactAttributes(record.attributes))) {
      record.setAttribute(key, value as string);
    }
  }

  forceFlush(): Promise<void> {
    return Promise.resolve();
  }

  shutdown(): Promise<void> {
    return Promise.resolve();
  }
}
