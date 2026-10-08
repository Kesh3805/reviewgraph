import type { NestExpressApplication } from '@nestjs/platform-express';
import RedisMock from 'ioredis-mock';
import request from 'supertest';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import type { NormalizeResult, ProviderEvent } from '../../src/providers/ports';
import { GithubPermissionsMonitor } from '../../src/providers/github/permissions-monitor';
import {
  DefaultRepositorySettings,
  REPOSITORY_SETTINGS,
} from '../../src/repositories/repository-settings.port';
import { eventTimestamp, rateKey } from '../../src/webhooks/replay-guard';
import { signWebhookBody } from '../../src/webhooks/signature';
import {
  DELIVERY_STORE,
  EVENT_NORMALIZER,
  PROVIDER_EVENT_SINK,
  type DeliveryContext,
  type DeliveryOutcome,
  type DeliveryRecord,
  type DeliveryWorkResult,
} from '../../src/webhooks/webhook.ports';
import { createTestApp } from '../helpers';
import { generateAppKey } from '../helpers/fake-github';
import { MemoryDeliveryStore } from '../helpers/memory-delivery-store';

const SECRET = 'whsec_replay_0123456789';
const URL = '/api/v1/webhooks/github';
const KEY = generateAppKey();

/** The memory store plus the recorded body hash per delivery id (as webhook_deliveries keeps). */
class HashingMemoryStore extends MemoryDeliveryStore {
  readonly hashes = new Map<string, string>();

  override async process<A>(
    delivery: DeliveryRecord,
    work: (ctx: DeliveryContext) => Promise<DeliveryWorkResult<A>>,
  ): Promise<DeliveryOutcome<A>> {
    const outcome = await super.process(delivery, work);
    if (!outcome.duplicate) this.hashes.set(delivery.deliveryId, delivery.payloadSha256);
    return outcome;
  }

  payloadHashOf(deliveryId: string): Promise<string | null> {
    return Promise.resolve(this.hashes.get(deliveryId) ?? null);
  }
}

const EVENT: ProviderEvent = {
  type: 'pull_request_closed',
  provider: 'github',
  deliveryId: 'x',
  installationId: '99',
  repo: { provider: 'github', installationId: '99', owner: 'o', name: 'r' },
  pr: { provider: 'github', installationId: '99', owner: 'o', name: 'r', number: 7 },
  merged: false,
};

const body = (installation: number, updatedAt?: string, note = 'a'): string =>
  JSON.stringify({
    action: 'closed',
    installation: { id: installation },
    note,
    ...(updatedAt ? { pull_request: { updated_at: updatedAt } } : {}),
  });

describe('webhook replay guard (SEC-006)', () => {
  let app: NestExpressApplication;
  let store: HashingMemoryStore;
  let redis: InstanceType<typeof RedisMock>;
  const dispatched: ProviderEvent[] = [];

  async function start(env: NodeJS.ProcessEnv = {}): Promise<void> {
    store = new HashingMemoryStore();
    app = await createTestApp(undefined, [], {
      redis,
      env: {
        GITHUB_ENABLED: 'true',
        GITHUB_APP_ID: '1',
        GITHUB_APP_PRIVATE_KEY: KEY.pem.replace(/\n/g, '\\n'),
        GITHUB_WEBHOOK_SECRET: SECRET,
        GITHUB_CLIENT_ID: 'c',
        GITHUB_CLIENT_SECRET: 's',
        ...env,
      },
      configure: (b) =>
        b
          .overrideProvider(EVENT_NORMALIZER)
          .useValue({ normalize: (): Promise<NormalizeResult> => Promise.resolve(EVENT) })
          .overrideProvider(GithubPermissionsMonitor)
          .useValue({ enabled: false, status: () => 'unknown' })
          .overrideProvider(PROVIDER_EVENT_SINK)
          .useValue({
            dispatch: (e: ProviderEvent) => {
              dispatched.push(e);
              return Promise.resolve();
            },
          })
          .overrideProvider(DELIVERY_STORE)
          .useValue(store)
          .overrideProvider(REPOSITORY_SETTINGS)
          .useValue(new DefaultRepositorySettings()),
    });
    await app.init();
  }

  beforeEach(() => {
    redis = new RedisMock();
    dispatched.length = 0;
    resetCounterTotals();
  });
  afterEach(async () => {
    await app?.close();
    await redis.flushall();
  });

  const post = (raw: string, deliveryId = 'd-1', signature?: string) =>
    request(app.getHttpServer())
      .post(URL)
      .set('content-type', 'application/json')
      .set('x-github-event', 'pull_request')
      .set('x-github-delivery', deliveryId)
      .set('x-hub-signature-256', signature ?? signWebhookBody(SECRET, Buffer.from(raw)))
      .send(raw);

  it('duplicate_delivery_same_body_is_noop_202', async () => {
    await start();
    const raw = body(1);
    expect((await post(raw)).body).toEqual({ delivery_id: 'd-1', accepted: true });
    const again = await post(raw);
    expect(again.status).toBe(202);
    expect(again.body).toEqual({ delivery_id: 'd-1', accepted: false, reason: 'duplicate' });
    expect(dispatched).toHaveLength(1);
  });

  it('same_delivery_id_different_body_flagged', async () => {
    await start();
    await post(body(1, undefined, 'original'));
    const replay = await post(body(1, undefined, 'substituted'));
    expect(replay.status).toBe(202);
    expect(replay.body).toEqual({
      delivery_id: 'd-1',
      accepted: false,
      reason: 'delivery_id_reuse',
    });
    expect(dispatched).toHaveLength(1);
    expect(counterTotal('webhook_replays_rejected_total', { reason: 'delivery_id_reuse' })).toBe(1);
  });

  it('stale_event_accepted_but_ignored', async () => {
    await start();
    const old = new Date(Date.now() - 3600_000).toISOString();
    const res = await post(body(1, old), 'd-old');
    expect(res.status).toBe(202);
    expect(res.body).toEqual({ delivery_id: 'd-old', accepted: false, reason: 'stale_event' });
    expect(dispatched).toEqual([]);
    expect(counterTotal('webhook_stale_events_total')).toBe(1);
    // Recorded: a replay of it is a plain duplicate.
    expect((await post(body(1, old), 'd-old')).body.reason).toBe('duplicate');
    // A recent event is processed.
    const fresh = await post(body(1, new Date().toISOString()), 'd-new');
    expect(fresh.body.accepted).toBe(true);
  });

  it('fresh_event_processed_once_under_concurrent_duplicates', async () => {
    await start();
    const raw = body(1, new Date().toISOString());
    const responses = await Promise.all(Array.from({ length: 10 }, () => post(raw, 'd-conc')));
    expect(responses.every((r) => r.status === 202)).toBe(true);
    expect(responses.filter((r) => r.body.accepted === true)).toHaveLength(1);
    expect(dispatched).toHaveLength(1);
  });

  it('guard_runs_only_after_valid_signature', async () => {
    await start({ WEBHOOK_INSTALLATION_RATE_LIMIT: '1' });
    const raw = body(77);
    for (let i = 0; i < 3; i++) {
      expect((await post(raw, `bad-${i}`, `sha256=${'0'.repeat(64)}`)).status).toBe(401);
    }
    expect(await redis.keys('rg:wh:rate:*')).toEqual([]);
    expect((await post(raw, 'good')).status).toBe(202);
  });

  it('rate_limit_returns_429_per_installation', async () => {
    await start({ WEBHOOK_INSTALLATION_RATE_LIMIT: '3' });
    for (let i = 0; i < 3; i++) expect((await post(body(5), `r-${i}`)).status).toBe(202);
    const limited = await post(body(5), 'r-3');
    expect(limited.status).toBe(429);
    expect(Number(limited.headers['retry-after'])).toBeGreaterThan(0);
    expect(limited.text).toBe('');
    // Another installation is unaffected.
    expect((await post(body(6), 'r-other')).status).toBe(202);
    expect(counterTotal('webhook_replays_rejected_total', { reason: 'rate_limited' })).toBe(1);
  });

  it('redis_down_falls_back_to_table', async () => {
    (redis as unknown as { incr: () => Promise<number> }).incr = () =>
      Promise.reject(new Error('redis down'));
    await start({ WEBHOOK_INSTALLATION_RATE_LIMIT: '1' });
    expect((await post(body(8), 'x-1')).status).toBe(202);
    expect((await post(body(8), 'x-2')).status).toBe(202);
    // Deduplication still holds without Redis.
    expect((await post(body(8), 'x-2')).body.reason).toBe('duplicate');
  });

  it('eventTimestamp picks the newest signed timestamp', () => {
    expect(eventTimestamp({ installation: {} })).toBeNull();
    expect(
      eventTimestamp({
        pull_request: { updated_at: '2026-10-01T00:00:00Z' },
        head_commit: { timestamp: '2026-10-02T00:00:00Z' },
      })?.toISOString(),
    ).toBe('2026-10-02T00:00:00.000Z');
    expect(eventTimestamp({ pull_request: { updated_at: 'garbage' } })).toBeNull();
    expect(rateKey('9', 3)).toBe('rg:wh:rate:9:3');
  });
});
