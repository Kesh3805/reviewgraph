import { createHmac } from 'node:crypto';
import crypto from 'node:crypto';
import { Logger } from '@nestjs/common';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { trace } from '@opentelemetry/api';
import {
  BasicTracerProvider,
  InMemorySpanExporter,
  SimpleSpanProcessor,
} from '@opentelemetry/sdk-trace-base';
import request from 'supertest';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import type { NormalizeResult, ProviderEvent } from '../../src/providers/ports';
import { signWebhookBody } from '../../src/webhooks/signature';
import {
  DELIVERY_STORE,
  EVENT_NORMALIZER,
  PROVIDER_EVENT_SINK,
  type DeliveryStore,
} from '../../src/webhooks/webhook.ports';
import { createTestApp } from '../helpers';
import { generateAppKey } from '../helpers/fake-github';

const SECRET = 'whsec_current_0123456789';
const PREVIOUS = 'whsec_previous_0123456789';
const URL = '/api/v1/webhooks/github';
const KEY = generateAppKey();

const GITHUB_ENV = {
  GITHUB_ENABLED: 'true',
  GITHUB_APP_ID: '12345',
  GITHUB_APP_PRIVATE_KEY: KEY.pem.replace(/\n/g, '\\n'),
  GITHUB_WEBHOOK_SECRET: SECRET,
  GITHUB_WEBHOOK_SECRET_PREVIOUS: PREVIOUS,
  GITHUB_CLIENT_ID: 'client',
  GITHUB_CLIENT_SECRET: 'client-secret',
};

const EVENT: ProviderEvent = {
  type: 'pull_request_closed',
  provider: 'github',
  deliveryId: 'd-1',
  installationId: '99',
  repo: { provider: 'github', installationId: '99', owner: 'o', name: 'r' },
  pr: { provider: 'github', installationId: '99', owner: 'o', name: 'r', number: 7 },
  merged: false,
};

const PAYLOAD = '{ "action": "opened",  "installation": {"id": 99},\n "note": "héllo wörld"  }';

describe('GitHub webhook endpoint (e2e)', () => {
  let app: NestExpressApplication;
  const dispatched: ProviderEvent[] = [];
  let normalized: NormalizeResult = EVENT;
  let sinkDelayMs = 0;
  let store: DeliveryStore | undefined;
  const exporter = new InMemorySpanExporter();

  beforeAll(() => {
    trace.setGlobalTracerProvider(
      new BasicTracerProvider({ spanProcessors: [new SimpleSpanProcessor(exporter)] }),
    );
  });

  async function start(options: Parameters<typeof createTestApp>[2] = {}): Promise<void> {
    app = await createTestApp(undefined, [], {
      env: GITHUB_ENV,
      ...options,
      configure: (builder) => {
        builder
          .overrideProvider(EVENT_NORMALIZER)
          .useValue({ normalize: () => Promise.resolve(normalized) })
          .overrideProvider(PROVIDER_EVENT_SINK)
          .useValue({
            dispatch: async (event: ProviderEvent) => {
              dispatched.push(event);
              if (sinkDelayMs) await new Promise((r) => setTimeout(r, sinkDelayMs));
            },
          });
        if (store) builder.overrideProvider(DELIVERY_STORE).useValue(store);
        return builder;
      },
    });
    await app.init();
  }

  beforeEach(() => {
    dispatched.length = 0;
    normalized = EVENT;
    sinkDelayMs = 0;
    store = undefined;
    exporter.reset();
    resetCounterTotals();
  });
  afterEach(async () => {
    await app?.close();
  });

  const post = (
    body: string | Buffer,
    headers: Record<string, string | undefined> = {},
    secret = SECRET,
  ) => {
    const buf = Buffer.from(body);
    const req = request(app.getHttpServer())
      .post(URL)
      .set('content-type', 'application/json')
      .set('x-github-event', headers['x-github-event'] ?? 'pull_request')
      .set('x-github-delivery', headers['x-github-delivery'] ?? 'd-1')
      .set('x-hub-signature-256', headers['x-hub-signature-256'] ?? signWebhookBody(secret, buf));
    return req.send(buf.toString('utf8'));
  };

  it('valid_signature_202: signature is checked over the raw bytes, not re-serialized JSON', async () => {
    await start();
    const res = await post(PAYLOAD);
    expect(res.status).toBe(202);
    expect(res.body).toEqual({ delivery_id: 'd-1', accepted: true });
    expect(dispatched).toEqual([EVENT]);
    expect(counterTotal('webhooks_received_total', { event: 'pull_request', accepted: true })).toBe(
      1,
    );
  });

  it('invalid_signature_401: tampered payload, empty body, metric incremented', async () => {
    await start();
    const signed = signWebhookBody(SECRET, Buffer.from(PAYLOAD));
    const res = await post(`${PAYLOAD} `, { 'x-hub-signature-256': signed });
    expect(res.status).toBe(401);
    expect(res.text).toBe('');
    expect(dispatched).toEqual([]);
    expect(counterTotal('webhook_signature_failures_total', { reason: 'mismatch' })).toBe(1);

    const wrongSecret = await post(PAYLOAD, {}, 'some-other-secret');
    expect(wrongSecret.status).toBe(401);
  });

  it('missing signature header is 401', async () => {
    await start();
    const res = await request(app.getHttpServer())
      .post(URL)
      .set('content-type', 'application/json')
      .set('x-github-event', 'pull_request')
      .set('x-github-delivery', 'd-1')
      .send(PAYLOAD);
    expect(res.status).toBe(401);
    expect(counterTotal('webhook_signature_failures_total', { reason: 'missing' })).toBe(1);
  });

  it('length_mismatch_401_without_compare', async () => {
    await start();
    const spy = jest.spyOn(crypto, 'timingSafeEqual');
    try {
      for (const bad of [
        'sha256=abcd',
        `sha256=${'a'.repeat(62)}`,
        `sha256=${'a'.repeat(66)}`,
        'sha1=abc',
        'abc',
      ]) {
        const res = await post(PAYLOAD, { 'x-hub-signature-256': bad });
        expect(res.status).toBe(401);
      }
      expect(spy).not.toHaveBeenCalled();
      // A full-length wrong digest does reach the constant-time compare.
      await post(PAYLOAD, { 'x-hub-signature-256': `sha256=${'a'.repeat(64)}` });
      expect(spy).toHaveBeenCalled();
    } finally {
      spy.mockRestore();
    }
  });

  it('rotated_previous_secret_accepted', async () => {
    await start();
    const res = await post(PAYLOAD, {}, PREVIOUS);
    expect(res.status).toBe(202);
    expect(res.body.accepted).toBe(true);
  });

  it('unsupported_event_202_not_accepted (and ping)', async () => {
    await start();
    const star = await post(PAYLOAD, { 'x-github-event': 'star', 'x-github-delivery': 'd-star' });
    expect(star.status).toBe(202);
    expect(star.body).toEqual({
      delivery_id: 'd-star',
      accepted: false,
      reason: 'unsupported_event',
    });
    const ping = await post('{"zen":"hi"}', {
      'x-github-event': 'ping',
      'x-github-delivery': 'd-ping',
    });
    expect(ping.status).toBe(202);
    expect(ping.body.accepted).toBe(false);
    expect(dispatched).toEqual([]);
  });

  it('ignored events are acknowledged with their reason', async () => {
    normalized = { ignored: true, reason: 'draft' };
    await start();
    const res = await post(PAYLOAD);
    expect(res.status).toBe(202);
    expect(res.body).toEqual({ delivery_id: 'd-1', accepted: false, reason: 'draft' });
    expect(dispatched).toEqual([]);
  });

  it('a redelivery is acknowledged as a duplicate and not dispatched twice', async () => {
    await start();
    expect((await post(PAYLOAD)).body.accepted).toBe(true);
    const again = await post(PAYLOAD);
    expect(again.status).toBe(202);
    expect(again.body).toEqual({ delivery_id: 'd-1', accepted: false, reason: 'duplicate' });
    expect(dispatched).toHaveLength(1);
  });

  it('db_down_503: GitHub retries when the delivery cannot be recorded', async () => {
    store = { record: () => Promise.reject(new Error('connection refused')) };
    await start();
    const res = await post(PAYLOAD);
    expect(res.status).toBe(503);
    expect(dispatched).toEqual([]);
  });

  it('ack_under_500ms: a slow orchestrator does not delay the acknowledgement', async () => {
    sinkDelayMs = 2000;
    await start();
    const started = Date.now();
    const res = await post(PAYLOAD);
    const elapsed = Date.now() - started;
    expect(res.status).toBe(202);
    expect(res.body.accepted).toBe(true);
    expect(elapsed).toBeLessThan(500);
    expect(dispatched).toHaveLength(1);
  });

  it('rejects missing event/delivery headers and invalid JSON with 400 after verification', async () => {
    await start();
    const buf = Buffer.from(PAYLOAD);
    const noHeaders = await request(app.getHttpServer())
      .post(URL)
      .set('content-type', 'application/json')
      .set('x-hub-signature-256', signWebhookBody(SECRET, buf))
      .send(buf.toString('utf8'));
    expect(noHeaders.status).toBe(400);
    const notJson = await post('not json at all');
    expect(notJson.status).toBe(400);
  });

  it('returns 413 above the raw body limit', async () => {
    await start({ webhookBodyLimit: '1kb' });
    const res = await post(JSON.stringify({ blob: 'x'.repeat(4096) }));
    expect(res.status).toBe(413);
  });

  it('is not served when GitHub is disabled', async () => {
    await start({ env: { GITHUB_ENABLED: 'false' } });
    const res = await post(PAYLOAD);
    expect(res.status).toBe(404);
  });

  it('starts a webhook_received span and logs no payload content', async () => {
    const lines: string[] = [];
    const spies = (['log', 'warn', 'error'] as const).map((level) =>
      jest.spyOn(Logger.prototype, level).mockImplementation((...args: unknown[]) => {
        lines.push(args.map(String).join(' '));
      }),
    );
    try {
      await start();
      await post(PAYLOAD);
      await post(`${PAYLOAD} `, {
        'x-hub-signature-256': signWebhookBody(SECRET, Buffer.from(PAYLOAD)),
      });
    } finally {
      spies.forEach((s) => s.mockRestore());
    }
    const span = exporter.getFinishedSpans().find((s) => s.name === 'webhook_received');
    expect(span?.attributes).toMatchObject({
      event: 'pull_request',
      action: 'opened',
      delivery_id: 'd-1',
    });
    const output = lines.join('\n');
    expect(output).toContain('delivery=d-1');
    expect(output).toContain('installation=99');
    expect(output).not.toContain('héllo');
    expect(output).not.toContain(SECRET);
  });

  it('createHmac reference: header format is sha256=<hex>', () => {
    const body = Buffer.from('x');
    expect(signWebhookBody('k', body)).toBe(
      `sha256=${createHmac('sha256', 'k').update(body).digest('hex')}`,
    );
  });
});
