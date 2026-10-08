import { randomUUID } from 'node:crypto';
import request from 'supertest';
import type { NormalizeResult } from '../src/providers/ports';
import { deliveryKey } from '../src/webhooks/delivery-store';
import { signWebhookBody } from '../src/webhooks/signature';
import { EVENT_NORMALIZER, PROVIDER_EVENT_SINK } from '../src/webhooks/webhook.ports';
import { type Harness, WEBHOOK_SECRET, seedRepo, startHarness } from './github-harness';

describe('webhook replay protection (integration, SEC-006)', () => {
  let h: Harness;
  let failNormalize = false;
  const deliveries: string[] = [];

  beforeAll(async () => {
    h = await startHarness({
      configure: (b) =>
        b
          .overrideProvider(EVENT_NORMALIZER)
          .useValue({
            normalize: (): Promise<NormalizeResult> =>
              failNormalize
                ? Promise.reject(new Error('normalizer down'))
                : Promise.resolve({ ignored: true, reason: 'unsupported_action' }),
          })
          .overrideProvider(PROVIDER_EVENT_SINK)
          .useValue({ dispatch: () => Promise.resolve() }),
    });
  });
  afterAll(async () => {
    if (deliveries.length) {
      await h.admin
        .deleteFrom('webhook_deliveries')
        .where('delivery_id', 'in', deliveries)
        .execute();
      await h.redis.del(...deliveries.map(deliveryKey));
    }
    await h.close();
  });
  beforeEach(() => {
    failNormalize = false;
  });

  const post = (raw: string, deliveryId: string) =>
    request(h.app.getHttpServer())
      .post('/api/v1/webhooks/github')
      .set('content-type', 'application/json')
      .set('x-github-event', 'pull_request')
      .set('x-github-delivery', deliveryId)
      .set('x-hub-signature-256', signWebhookBody(WEBHOOK_SECRET, Buffer.from(raw)))
      .send(raw);

  const newDelivery = () => {
    const id = `sec006-${randomUUID()}`;
    deliveries.push(id);
    return id;
  };

  it('replay_after_redis_ttl_still_detected_via_table; same_delivery_id_different_body_flagged_and_audited', async () => {
    const s = await seedRepo(h);
    const id = newDelivery();
    const raw = JSON.stringify({
      action: 'labeled',
      installation: { id: Number(s.installationId) },
    });
    expect((await post(raw, id)).status).toBe(202);
    // The Redis fast-path key expired: Postgres still knows the delivery.
    await h.redis.del(deliveryKey(id));
    expect((await post(raw, id)).body.reason).toBe('duplicate');

    const substituted = JSON.stringify({
      action: 'labeled',
      installation: { id: Number(s.installationId) },
      injected: true,
    });
    const replay = await post(substituted, id);
    expect(replay.status).toBe(202);
    expect(replay.body.reason).toBe('delivery_id_reuse');
    const audit = await h.admin
      .selectFrom('audit_log')
      .select(['action', 'outcome', 'target_id'])
      .where('organization_id', '=', s.org.organizationId)
      .execute();
    expect(audit).toEqual([
      { action: 'webhook.delivery_id_reuse', outcome: 'denied', target_id: id },
    ]);
  });

  it('operator_redelivery_of_failed_delivery_is_processed', async () => {
    const id = newDelivery();
    const raw = JSON.stringify({ action: 'labeled', installation: { id: 1 } });
    failNormalize = true;
    expect((await post(raw, id)).status).toBe(503);
    failNormalize = false;
    const redelivered = await post(raw, id);
    expect(redelivered.status).toBe(202);
    expect(redelivered.body.reason).toBe('unsupported_action');
  });
});
