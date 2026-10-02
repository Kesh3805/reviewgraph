import { z } from 'zod';
import { ServiceKeyRing } from '../internal/service-token';
import { telemetryEnvShape } from '../telemetry/config';

/** Exit code for configuration errors (sysexits EX_CONFIG). */
export const EX_CONFIG = 78;

const boolFromString = z
  .enum(['true', 'false', '1', '0'])
  .transform((v) => v === 'true' || v === '1');

const nonEmpty = z.string().min(1);

const secretMin32Bytes = z
  .string()
  .refine((v) => Buffer.byteLength(v, 'utf8') >= 32, 'must be at least 32 bytes');

const tokenCacheKey = z.string().refine((v) => Buffer.from(v, 'base64').length === 32, {
  message: 'must be 32 bytes, base64 encoded',
});

const serviceJwtKeys = z.string().refine(
  (v) => {
    try {
      ServiceKeyRing.parse(v);
      return true;
    } catch {
      return false;
    }
  },
  { message: 'must be kid:base64[,kid:base64...] with keys of at least 32 bytes' },
);

const url = (protocols: string[]) =>
  z.string().refine(
    (v) => {
      try {
        return protocols.includes(new URL(v).protocol);
      } catch {
        return false;
      }
    },
    { message: `must be a valid URL (${protocols.join(', ')})` },
  );

const baseShape = {
  NODE_ENV: z.enum(['development', 'test', 'production']).default('development'),
  PORT: z.coerce.number().int().min(1).max(65535).default(8080),
  DATABASE_URL: url(['postgres:', 'postgresql:']),
  REDIS_URL: url(['redis:', 'rediss:']),
  ENGINE_INTERNAL_URL: url(['http:', 'https:']),
  SERVICE_JWT_SECRET: secretMin32Bytes,
  /** `kid1:base64,kid2:base64`; the first key signs, all verify. Defaults to SERVICE_JWT_SECRET. */
  SERVICE_JWT_KEYS: serviceJwtKeys.optional(),
  SESSION_JWT_SECRET: secretMin32Bytes,
  TOKEN_CACHE_KEY: tokenCacheKey,
  WEB_ORIGIN: url(['http:', 'https:']),
  GITHUB_ENABLED: boolFromString.default(true),
  GITHUB_APP_ID: nonEmpty.optional(),
  GITHUB_APP_PRIVATE_KEY: nonEmpty.optional(),
  GITHUB_APP_PRIVATE_KEY_FILE: nonEmpty.optional(),
  GITHUB_WEBHOOK_SECRET: nonEmpty.optional(),
  GITHUB_CLIENT_ID: nonEmpty.optional(),
  GITHUB_CLIENT_SECRET: nonEmpty.optional(),
  GITHUB_API_URL: url(['http:', 'https:']).default('https://api.github.com'),
  ...telemetryEnvShape,
};

const GITHUB_REQUIRED = [
  'GITHUB_APP_ID',
  'GITHUB_WEBHOOK_SECRET',
  'GITHUB_CLIENT_ID',
  'GITHUB_CLIENT_SECRET',
] as const;

export const envSchema = z.object(baseShape).superRefine((env, ctx) => {
  if (!env.GITHUB_ENABLED) return;
  for (const key of GITHUB_REQUIRED) {
    if (!env[key]) {
      ctx.addIssue({
        code: 'custom',
        path: [key],
        message: 'required when GITHUB_ENABLED is true',
      });
    }
  }
  if (!env.GITHUB_APP_PRIVATE_KEY && !env.GITHUB_APP_PRIVATE_KEY_FILE) {
    ctx.addIssue({
      code: 'custom',
      path: ['GITHUB_APP_PRIVATE_KEY'],
      message: 'GITHUB_APP_PRIVATE_KEY or GITHUB_APP_PRIVATE_KEY_FILE is required',
    });
  }
});

export type Env = z.infer<typeof envSchema>;

export class ConfigError extends Error {
  constructor(readonly issues: string[]) {
    super(`Invalid configuration:\n${issues.map((i) => `  - ${i}`).join('\n')}`);
    this.name = 'ConfigError';
  }
}

/**
 * Validates raw environment variables. Issue messages name the variable and the rule that
 * failed but never echo the supplied value, so secrets cannot leak into logs.
 */
export function parseEnv(raw: NodeJS.ProcessEnv): Env {
  const result = envSchema.safeParse(raw);
  if (result.success) return result.data;
  throw new ConfigError(
    result.error.issues.map(
      (issue) => `${issue.path.join('.') || '(root)'}: ${redactedMessage(issue)}`,
    ),
  );
}

function redactedMessage(issue: z.core.$ZodIssue): string {
  // Zod messages for enum/literal mismatches can quote the received input; use fixed text.
  if (issue.code === 'invalid_value') return 'invalid value (value redacted)';
  if (issue.code === 'invalid_type' && issue.input === undefined) return 'is required';
  return issue.message.replace(/"[^"]*"/g, '"<redacted>"');
}
