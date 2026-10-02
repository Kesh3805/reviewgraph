import { ConfigError, parseEnv } from '../../src/config/env.schema';
import { VALID_ENV } from '../helpers';

function parseError(env: NodeJS.ProcessEnv): ConfigError {
  try {
    parseEnv(env);
  } catch (err) {
    return err as ConfigError;
  }
  throw new Error('expected parseEnv to throw');
}

describe('env schema', () => {
  it('env_schema_accepts_valid_env_and_applies_defaults', () => {
    const env = parseEnv({ ...VALID_ENV, PORT: undefined });
    expect(env.PORT).toBe(8080);
    expect(env.GITHUB_ENABLED).toBe(false);
    expect(env.GITHUB_API_URL).toBe('https://api.github.com');
    expect(env.OTEL_EXPORTER_OTLP_ENDPOINT).toBeUndefined();
  });

  it('env_schema_rejects_missing_database_url', () => {
    const err = parseError({ ...VALID_ENV, DATABASE_URL: undefined });
    expect(err).toBeInstanceOf(ConfigError);
    expect(err.issues.some((i) => i.startsWith('DATABASE_URL:'))).toBe(true);
  });

  it('env_schema_redacts_values_in_errors', () => {
    const secret = 'super-secret-value';
    const err = parseError({
      ...VALID_ENV,
      SERVICE_JWT_SECRET: secret, // too short
      NODE_ENV: secret, // invalid enum
      DATABASE_URL: secret, // not a URL
      TOKEN_CACHE_KEY: secret,
    });
    expect(err.issues.length).toBeGreaterThanOrEqual(4);
    expect(err.message).not.toContain(secret);
  });

  it('env_schema_requires_github_vars_only_when_enabled', () => {
    expect(() => parseEnv({ ...VALID_ENV, GITHUB_ENABLED: 'true' })).toThrow(ConfigError);
    const err = parseError({ ...VALID_ENV, GITHUB_ENABLED: 'true' });
    expect(err.issues.some((i) => i.startsWith('GITHUB_APP_ID:'))).toBe(true);
    expect(() =>
      parseEnv({
        ...VALID_ENV,
        GITHUB_ENABLED: 'true',
        GITHUB_APP_ID: '1',
        GITHUB_APP_PRIVATE_KEY_FILE: '/run/secrets/key.pem',
        GITHUB_WEBHOOK_SECRET: 'w',
        GITHUB_CLIENT_ID: 'c',
        GITHUB_CLIENT_SECRET: 'x',
      }),
    ).not.toThrow();
  });

  it('env_schema_rejects_short_service_secret', () => {
    const err = parseError({ ...VALID_ENV, SERVICE_JWT_SECRET: 'short' });
    expect(err.issues.some((i) => i.startsWith('SERVICE_JWT_SECRET:'))).toBe(true);
  });
});
