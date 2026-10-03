import { randomUUID } from 'node:crypto';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { Redis } from 'ioredis';
import request from 'supertest';
import { counterTotal, resetCounterTotals } from '../src/common/metrics';
import { DbService } from '../src/db/db.module';
import { createKysely, createPool } from '../src/db/kysely.provider';
import type { ProviderEvent } from '../src/providers/ports';
import { GithubPermissionsMonitor } from '../src/providers/github/permissions-monitor';
import { deliveryKey, PgDeliveryStore } from '../src/webhooks/delivery-store';
import { signWebhookBody } from '../src/webhooks/signature';
import {
  EVENT_NORMALIZER,
  PROVIDER_EVENT_SINK,
  type DeliveryRecord,
  type DeliveryWorkResult,
} from '../src/webhooks/webhook.ports';
import { createTestApp } from '../test/helpers';
import { generateAppKey } from '../test/helpers/fake-github';
import { adminDb } from './seed';

const sha = 'a'.repeat(64);
const newId = (): string => `it-${randomUUID()}`;
const record = (deliveryId: string, extra: Partial<DeliveryRecord> = {}): DeliveryRecord => ({
  deliveryId,
  eventName: 'pull_request',
  action: 'opened',
  installationId: '12345',
  payloadSha256: sha,
  ...extra,
});
const processed = (): DeliveryWorkResult<string> => ({ status: 'processed', ack: 'ok' });

describe('webhook delivery idempotency (integration)', () => {
  const admin = adminDb();
  const redis = new Redis(process.env.RG_TEST_REDIS_URL!);
  const appDb = createKysely(
    createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max: 12 }),
  );
  const dbs = new DbService(appDb, { DB_APP_ROLE: 'rg_api' } as never);
  const store = new PgDeliveryStore(dbs, redis);
  const created: string[] = [];

  const rowsFor = async (id: string) =>
    admin.selectFrom('webhook_deliveries').selectAll().where('delivery_id', '=', id).execute();
  const fresh = (): string => {
    const id = newId();
    created.push(id);
    return id;
  };

  afterAll(async () => {
    if (created.length) {
      await admin.deleteFrom('webhook_deliveries').where('delivery_id', 'in', created).execute();
      await redis.del(...created.map(deliveryKey));
    }
    await appDb.destroy();
    await admin.destroy();
    redis.disconnect();
  });
  beforeEach(() => resetCounterTotals());

  it('same_delivery_twice_one_run', async () => {
    const id = fresh();
    let runs = 0;
    const work = () => {
      runs++;
      return Promise.resolve(processed());
    };
    const first = await store.process(record(id), work);
    const second = await store.process(record(id), work);
    expect(first).toEqual({ duplicate: false, ack: 'ok' });
    expect(second).toEqual({ duplicate: true });
    expect(runs).toBe(1);
    expect(counterTotal('webhook_duplicates_total')).toBe(1);

    const rows = await rowsFor(id);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      provider: 'github',
      event: 'pull_request',
      action: 'opened',
      status: 'processed',
      signature_valid: true,
      payload_sha256: sha,
    });
    expect(rows[0]!.provider_installation_id).toBe('12345');
    expect(rows[0]!.processed_at).not.toBeNull();
    expect(await redis.ttl(deliveryKey(id))).toBeGreaterThan(259_000);
  });

  it('records an ignored outcome', async () => {
    const id = fresh();
    await store.process(record(id), () => Promise.resolve({ status: 'ignored', ack: 'x' }));
    expect((await rowsFor(id))[0]!.status).toBe('ignored');
  });

  it('concurrent_same_delivery_one_row', async () => {
    const id = fresh();
    let runs = 0;
    const outcomes = await Promise.all(
      Array.from({ length: 10 }, () =>
        store.process(record(id), async () => {
          runs++;
          await new Promise((resolve) => setTimeout(resolve, 80));
          return processed();
        }),
      ),
    );
    expect(outcomes.filter((o) => !o.duplicate)).toHaveLength(1);
    expect(outcomes.filter((o) => o.duplicate)).toHaveLength(9);
    expect(runs).toBe(1);
    expect(await rowsFor(id)).toHaveLength(1);
  });

  it('redis_seen_pg_missing_processes', async () => {
    const id = fresh();
    // A crash after the Redis write and before the commit leaves exactly this state.
    await redis.set(deliveryKey(id), '1', 'EX', 3600);
    let runs = 0;
    const outcome = await store.process(record(id), () => {
      runs++;
      return Promise.resolve(processed());
    });
    expect(outcome).toEqual({ duplicate: false, ack: 'ok' });
    expect(runs).toBe(1);
    expect(await rowsFor(id)).toHaveLength(1);
    // And now that PG has it, the Redis hit is confirmed as a duplicate.
    expect(await store.process(record(id), () => Promise.resolve(processed()))).toEqual({
      duplicate: true,
    });
  });

  it('rollback_allows_retry', async () => {
    const id = fresh();
    await expect(
      store.process(record(id), () => Promise.reject(new Error('handler blew up'))),
    ).rejects.toThrow('handler blew up');
    expect(await rowsFor(id)).toHaveLength(0);
    expect(await redis.exists(deliveryKey(id))).toBe(0);
    let runs = 0;
    const retry = await store.process(record(id), () => {
      runs++;
      return Promise.resolve(processed());
    });
    expect(retry).toEqual({ duplicate: false, ack: 'ok' });
    expect(runs).toBe(1);
    expect(await rowsFor(id)).toHaveLength(1);
  });

  it('a failed finish step also rolls the record back', async () => {
    const id = fresh();
    await expect(
      store.process(record(id), () =>
        Promise.resolve({
          status: 'processed' as const,
          ack: 'x',
          // Not an organization: the finish step fails its foreign key, after the insert.
          organizationId: randomUUID(),
        }),
      ),
    ).rejects.toThrow();
    expect(await rowsFor(id)).toHaveLength(0);
  });

  it('redis_down_still_deduplicates', async () => {
    const dead = new Redis('redis://127.0.0.1:1', {
      lazyConnect: true,
      maxRetriesPerRequest: 0,
      enableOfflineQueue: false,
      retryStrategy: () => null,
    });
    dead.on('error', () => undefined);
    const pgOnly = new PgDeliveryStore(dbs, dead);
    const id = fresh();
    let runs = 0;
    const work = () => {
      runs++;
      return Promise.resolve(processed());
    };
    expect(await pgOnly.process(record(id), work)).toEqual({ duplicate: false, ack: 'ok' });
    expect(await pgOnly.process(record(id), work)).toEqual({ duplicate: true });
    expect(runs).toBe(1);
    expect(await rowsFor(id)).toHaveLength(1);
    dead.disconnect();
  });

  it('stores only the payload hash, never a payload', async () => {
    const id = fresh();
    await store.process(record(id), () => Promise.resolve(processed()));
    const columns = Object.keys((await rowsFor(id))[0]!);
    expect(columns).not.toContain('payload');
    expect(columns).toContain('payload_sha256');
  });
});

describe('webhook endpoint with the real delivery store (integration)', () => {
  const SECRET = 'whsec_integration_0123456789';
  const key = generateAppKey();
  const admin = adminDb();
  // The app owns (and closes) `appRedis`; `redis` is for assertions and cleanup.
  const appRedis = new Redis(process.env.RG_TEST_REDIS_URL!);
  const redis = new Redis(process.env.RG_TEST_REDIS_URL!);
  const dispatched: ProviderEvent[] = [];
  const ids: string[] = [];
  let app: NestExpressApplication;

  beforeAll(async () => {
    app = await createTestApp(undefined, [], {
      redis: appRedis,
      env: {
        DATABASE_URL: process.env.RG_TEST_DATABASE_URL!,
        DB_APP_ROLE: 'rg_api',
        GITHUB_ENABLED: 'true',
        GITHUB_APP_ID: '1',
        GITHUB_APP_PRIVATE_KEY: key.pem.replace(/\n/g, '\\n'),
        GITHUB_WEBHOOK_SECRET: SECRET,
        GITHUB_CLIENT_ID: 'c',
        GITHUB_CLIENT_SECRET: 's',
      },
      configure: (builder) =>
        builder
          .overrideProvider(GithubPermissionsMonitor)
          .useValue({ enabled: false, status: () => 'unknown' })
          .overrideProvider(EVENT_NORMALIZER)
          .useValue({
            normalize: (_e: string, _p: unknown, deliveryId: string) =>
              Promise.resolve({
                type: 'pull_request_closed',
                provider: 'github',
                deliveryId,
                installationId: '99',
                repo: { provider: 'github', installationId: '99', owner: 'o', name: 'r' },
                pr: { provider: 'github', installationId: '99', owner: 'o', name: 'r', number: 7 },
                merged: false,
              } satisfies ProviderEvent),
          })
          .overrideProvider(PROVIDER_EVENT_SINK)
          .useValue({
            dispatch: (event: ProviderEvent) => {
              dispatched.push(event);
              return Promise.resolve();
            },
          }),
    });
    await app.init();
  });

  afterAll(async () => {
    await app.close();
    await admin.deleteFrom('webhook_deliveries').where('delivery_id', 'in', ids).execute();
    await redis.del(...ids.map(deliveryKey));
    await admin.destroy();
    redis.disconnect();
  });

  const send = (deliveryId: string) => {
    const body = Buffer.from('{"action":"closed","installation":{"id":99}}');
    return request(app.getHttpServer())
      .post('/api/v1/webhooks/github')
      .set('content-type', 'application/json')
      .set('x-github-event', 'pull_request')
      .set('x-github-delivery', deliveryId)
      .set('x-hub-signature-256', signWebhookBody(SECRET, body))
      .send(body.toString());
  };

  it('duplicate_webhook: 10 parallel replays produce exactly one accepted delivery', async () => {
    const id = newId();
    ids.push(id);
    const responses = await Promise.all(Array.from({ length: 10 }, () => send(id)));
    expect(responses.every((r) => r.status === 202)).toBe(true);
    const accepted = responses.filter((r) => r.body.accepted === true);
    const duplicates = responses.filter((r) => r.body.reason === 'duplicate');
    expect(accepted).toHaveLength(1);
    expect(duplicates).toHaveLength(9);
    expect(dispatched.filter((e) => e.deliveryId === id)).toHaveLength(1);
    const rows = await admin
      .selectFrom('webhook_deliveries')
      .select(['status', 'payload_sha256', 'provider_installation_id'])
      .where('delivery_id', '=', id)
      .execute();
    expect(rows).toHaveLength(1);
    expect(rows[0]!.status).toBe('processed');
    expect(rows[0]!.provider_installation_id).toBe('99');
  });

  it('a redelivery after a restart (Redis wiped) is still a duplicate', async () => {
    const id = newId();
    ids.push(id);
    expect((await send(id)).body.accepted).toBe(true);
    await redis.del(deliveryKey(id));
    const again = await send(id);
    expect(again.status).toBe(202);
    expect(again.body).toEqual({ delivery_id: id, accepted: false, reason: 'duplicate' });
    expect(dispatched.filter((e) => e.deliveryId === id)).toHaveLength(1);
  });
});
