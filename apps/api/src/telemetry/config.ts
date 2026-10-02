import { z } from 'zod';

const HEADER_NAME = /^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/;

/**
 * Parses `OTEL_EXPORTER_OTLP_HEADERS` (`k1=v1,k2=v2`, values percent-encoded per the OTel spec).
 * Throws on a malformed pair so a typo fails boot instead of silently dropping auth.
 */
export function parseOtlpHeaders(raw: string): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const pair of raw.split(',')) {
    const trimmed = pair.trim();
    if (!trimmed) continue;
    const eq = trimmed.indexOf('=');
    if (eq <= 0) throw new Error('malformed header pair (expected name=value)');
    const name = trimmed.slice(0, eq).trim();
    const value = trimmed.slice(eq + 1).trim();
    if (!HEADER_NAME.test(name)) throw new Error('invalid header name');
    try {
      headers[name] = decodeURIComponent(value);
    } catch {
      throw new Error('invalid percent-encoding in header value');
    }
  }
  return headers;
}

const otlpHeaders = z.string().refine(
  (v) => {
    try {
      parseOtlpHeaders(v);
      return true;
    } catch {
      return false;
    }
  },
  { message: 'must be comma separated name=value pairs' },
);

/** Telemetry-related environment variables (shared with the Rust services). */
export const telemetryEnvShape = {
  RG_OTEL_ENABLED: z
    .enum(['true', 'false', '1', '0'])
    .transform((v) => v === 'true' || v === '1')
    .default(true),
  OTEL_EXPORTER_OTLP_ENDPOINT: z
    .string()
    .refine(
      (v) => {
        try {
          return ['http:', 'https:'].includes(new URL(v).protocol);
        } catch {
          return false;
        }
      },
      { message: 'must be a valid URL (http:, https:)' },
    )
    .optional(),
  OTEL_EXPORTER_OTLP_HEADERS: otlpHeaders.optional(),
  OTEL_SERVICE_NAME: z.string().min(1).default('reviewgraph-api'),
};

export const telemetryEnvSchema = z.object(telemetryEnvShape);

export interface TelemetryConfig {
  /** True only when enabled and an OTLP endpoint is configured. */
  enabled: boolean;
  endpoint?: string;
  headers: Record<string, string>;
  serviceName: string;
  serviceVersion: string;
  environment: string;
}

export function toTelemetryConfig(
  env: z.infer<typeof telemetryEnvSchema>,
  nodeEnv = 'development',
): TelemetryConfig {
  const endpoint = env.OTEL_EXPORTER_OTLP_ENDPOINT?.replace(/\/+$/, '');
  return {
    enabled: env.RG_OTEL_ENABLED && Boolean(endpoint),
    endpoint,
    headers: env.OTEL_EXPORTER_OTLP_HEADERS ? parseOtlpHeaders(env.OTEL_EXPORTER_OTLP_HEADERS) : {},
    serviceName: env.OTEL_SERVICE_NAME,
    serviceVersion: process.env.npm_package_version ?? '0.1.0',
    environment: nodeEnv,
  };
}

/** Parses telemetry config from a raw environment; throws ZodError on invalid values. */
export function parseTelemetryConfig(raw: NodeJS.ProcessEnv): TelemetryConfig {
  return toTelemetryConfig(telemetryEnvSchema.parse(raw), raw.NODE_ENV);
}
