import { generateKeyPairSync } from 'node:crypto';
import { mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { Writable } from 'node:stream';
import { Logger } from '@nestjs/common';
import { trace } from '@opentelemetry/api';
import {
  BasicTracerProvider,
  InMemorySpanExporter,
  SimpleSpanProcessor,
} from '@opentelemetry/sdk-trace-base';
import RedisMock from 'ioredis-mock';
import { GithubAppAuth } from '../../src/providers/github/app-auth.service';
import { createGithubAppAuth } from '../../src/providers/github/github.module';
import { InstallationTokenCache } from '../../src/providers/github/token-cache';
import { GithubRepositoryProvider } from '../../src/providers/github/repository-provider';
import { checkCloneJob, type CloneJobGate } from '../../src/internal/clone-job-gate';
import {
  EnvSecretSource,
  FileSecretSource,
  SecretManagerSource,
  SecretSourceError,
  loadAppPrivateKey,
  parseAppPrivateKey,
  privateKeySource,
  scrubPrivateKeyFromEnv,
} from '../../src/security/secret-source';
import { createJsonLogger, JsonLoggerService } from '../../src/telemetry/json-logger';
import { testConfig } from '../helpers';
import { FakeGithub, generateAppKey } from '../helpers/fake-github';

const APP_KEY = generateAppKey();
const GITHUB_ON = {
  GITHUB_ENABLED: 'true',
  GITHUB_APP_ID: '1',
  GITHUB_WEBHOOK_SECRET: 'w',
  GITHUB_CLIENT_ID: 'c',
  GITHUB_CLIENT_SECRET: 's',
};

describe('SEC-005 private key handling', () => {
  it('private_key_missing_or_weak_fails_startup', async () => {
    const weak = generateKeyPairSync('rsa', { modulusLength: 1024 })
      .privateKey.export({ type: 'pkcs8', format: 'pem' })
      .toString();
    const ec = generateKeyPairSync('ec', { namedCurve: 'P-256' })
      .privateKey.export({ type: 'pkcs8', format: 'pem' })
      .toString();
    expect(() => parseAppPrivateKey(weak)).toThrow(/too weak \(1024 bits/);
    expect(() => parseAppPrivateKey(ec)).toThrow(/expected an RSA PEM/);
    expect(() => parseAppPrivateKey('nope')).toThrow(SecretSourceError);
    expect(parseAppPrivateKey(APP_KEY.pem).asymmetricKeyDetails?.modulusLength).toBe(2048);

    // Boot: a weak or missing key makes the GitHub provider factory throw.
    expect(() =>
      createGithubAppAuth(testConfig({ ...GITHUB_ON, GITHUB_APP_PRIVATE_KEY: weak }), new RedisMock()),
    ).toThrow(/too weak/);
    expect(() => testConfig(GITHUB_ON)).toThrow(/GITHUB_APP_PRIVATE_KEY/);
    expect(() => privateKeySource({})).toThrow(/not configured/);

    // Each source reports itself without the value.
    const dir = mkdtempSync(join(tmpdir(), 'rg-sec005-'));
    const file = join(dir, 'app.pem');
    writeFileSync(file, APP_KEY.pem);
    const fromFile = new FileSecretSource(file, 'GITHUB_APP_PRIVATE_KEY_FILE');
    expect(fromFile.description).toBe('file:GITHUB_APP_PRIVATE_KEY_FILE');
    await expect(loadAppPrivateKey(fromFile)).resolves.toBeDefined();
    await expect(
      loadAppPrivateKey(new FileSecretSource(join(dir, 'missing.pem'))),
    ).rejects.toThrow(/could not be read/);
    await expect(new EnvSecretSource('RG_NOT_SET_ANYWHERE', {}).load()).rejects.toThrow(
      'env:RG_NOT_SET_ANYWHERE is not set',
    );
    const manager = new SecretManagerSource(
      { access: (name) => Promise.reject(new Error(`backend said ${APP_KEY.pem} for ${name}`)) },
      'projects/p/secrets/github-app-key',
    );
    const failure = await manager.load().catch((e: unknown) => String(e));
    expect(failure).toContain('secret-manager:projects/p/secrets/github-app-key');
    expect(failure).not.toContain('PRIVATE KEY');
  });

  it('the inline key is removed from process.env after boot', () => {
    const env: NodeJS.ProcessEnv = { GITHUB_APP_PRIVATE_KEY: APP_KEY.pem, OTHER: 'x' };
    scrubPrivateKeyFromEnv(env);
    expect(env).toEqual({ OTHER: 'x' });
  });
});

describe('SEC-005 sentinel token leak scan', () => {
  let github: FakeGithub;
  beforeAll(async () => {
    github = new FakeGithub(APP_KEY.publicKey);
    await github.start();
  });
  afterAll(() => github.stop());

  it('sentinel_token_absent_from_logs_spans_db_and_redis_plaintext', async () => {
    const exporter = new InMemorySpanExporter();
    const provider = new BasicTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] });
    trace.setGlobalTracerProvider(provider);

    const lines: string[] = [];
    const sink = new Writable({
      write(chunk: Buffer, _enc, done) {
        lines.push(chunk.toString('utf8'));
        done();
      },
    });
    const logger = new JsonLoggerService(createJsonLogger('trace', sink));
    const spies = (['log', 'warn', 'error', 'debug', 'verbose'] as const).map((level) =>
      jest.spyOn(Logger.prototype, level).mockImplementation(function (
        this: Logger,
        message: unknown,
        ...rest: unknown[]
      ) {
        logger[level === 'verbose' ? 'verbose' : level](message, String(rest.at(-1) ?? ''));
      }),
    );

    const redis = new RedisMock();
    await redis.flushall();
    const auth = new GithubAppAuth({
      appId: '12345',
      privateKey: APP_KEY.privateKey,
      apiUrl: github.url,
      cache: new InstallationTokenCache(redis, Buffer.alloc(32, 9)),
      retries: false,
    });
    const provider_ = new GithubRepositoryProvider(
      testConfig({ GITHUB_API_URL: github.url }),
      auth,
      undefined as never,
      undefined as never,
    );
    let sentinel = '';
    try {
      // Webhook-to-clone path on the API side: mint, cache, broker credential, a failing call.
      const credential = await provider_.issueCloneCredential(
        { provider: 'github', installationId: '77', owner: 'acme', name: 'repo' },
        600,
        { providerRepoId: '4242' },
      );
      sentinel = credential.token.reveal();
      expect(sentinel).toMatch(/^ghs_/);
      // A cached read and an error path that sees the token in a request header.
      await auth.getInstallationToken('77', {
        repositoryIds: [4242],
        permissions: { contents: 'read' },
      });
      const octokit = await auth.getOctokit('77');
      await octokit.request('GET /repos/{owner}/{repo}', { owner: 'a', repo: 'b' }).catch((e) => {
        logger.error(e as Error, 'test');
        logger.warn(JSON.stringify(e));
      });
      logger.log({ credential }, 'test');
      logger.log(`credential ${String(credential.token)} ${JSON.stringify(credential)}`, 'test');
    } finally {
      spies.forEach((s) => s.mockRestore());
      await provider.forceFlush();
      trace.disable();
    }
    expect(sentinel).not.toBe('');

    const haystacks: [string, string][] = [['logs', lines.join('\n')]];
    for (const span of exporter.getFinishedSpans()) {
      haystacks.push([
        `span ${span.name}`,
        JSON.stringify({ attributes: span.attributes, events: span.events, status: span.status }),
      ]);
    }
    for (const key of await redis.keys('*')) {
      haystacks.push([`redis ${key}`, `${key} ${(await redis.get(key)) ?? ''}`]);
    }
    expect(haystacks.length).toBeGreaterThan(2);
    for (const [where, text] of haystacks) {
      expect({ where, leaked: text.includes(sentinel) }).toEqual({ where, leaked: false });
    }
  });

  it('no_credential_columns_in_schema (DB sink: tokens and keys have no column to land in)', () => {
    const dir = resolve(__dirname, '../../../../engine/migrations');
    const offenders: string[] = [];
    // Hashed or encrypted-at-rest columns that are allowed by name.
    const allowed = new Set(['session_hash', 'token_hash', 'secret_hash', 'secret_scan_status']);
    for (const file of readdirSync(dir).filter((f) => f.endsWith('.sql'))) {
      const sqlText = readFileSync(join(dir, file), 'utf8');
      for (const match of sqlText.matchAll(
        /^\s+([a-z_]*(?:token|secret|private_key|password)[a-z_]*)\s+(text|bytea|varchar|jsonb)/gim,
      )) {
        const column = match[1]!.toLowerCase();
        if (!allowed.has(column)) offenders.push(`${file}: ${column}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});

describe('SEC-005 broker job gate', () => {
  const REPO = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2c';
  const OTHER = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2d';
  const JOB = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a3e';
  const gate = (state: Awaited<ReturnType<CloneJobGate['lookup']>>): CloneJobGate => ({
    lookup: () => Promise.resolve(state),
  });

  it('broker_rejects_non_running_job_and_foreign_repo', async () => {
    expect(await checkCloneJob(gate({ state: 'running', repositoryId: REPO }), JOB, REPO)).toBe(
      'ok',
    );
    // Wrong state: 403.
    for (const state of ['queued', 'succeeded', 'failed', 'dead', 'cancelled']) {
      expect(await checkCloneJob(gate({ state, repositoryId: REPO }), JOB, REPO)).toBe(
        'not_running',
      );
    }
    // Unknown job or a job of another repository: 404, indistinguishable.
    expect(await checkCloneJob(gate(null), JOB, REPO)).toBe('not_found');
    expect(await checkCloneJob(gate({ state: 'running', repositoryId: OTHER }), JOB, REPO)).toBe(
      'not_found',
    );
    // No job claim, or no lookup wired yet: the repository claim alone scopes the credential.
    expect(await checkCloneJob(gate(null), undefined, REPO)).toBe('skipped');
    expect(await checkCloneJob(gate(undefined), JOB, REPO)).toBe('skipped');
  });
});
