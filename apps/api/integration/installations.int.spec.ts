import { randomInt, randomUUID } from 'node:crypto';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { Redis } from 'ioredis';
import request from 'supertest';
import { counterTotal, resetCounterTotals } from '../src/common/metrics';
import { JOB_CANCELLER } from '../src/common/job-canceller';
import { GithubPermissionsMonitor } from '../src/providers/github/permissions-monitor';
import {
  INSTALLATION_GATE,
  InstallationInactiveError,
  type InstallationGate,
} from '../src/providers/github/installation.service';
import { deliveryKey } from '../src/webhooks/delivery-store';
import { signWebhookBody } from '../src/webhooks/signature';
import { PROVIDER_EVENT_SINK } from '../src/webhooks/webhook.ports';
import { createTestApp } from '../test/helpers';
import { fixture } from '../test/helpers/fixtures';
import { generateAppKey } from '../test/helpers/fake-github';
import { adminDb } from './seed';

const SECRET = 'whsec_installations_0123456789';
const KEY = generateAppKey();

interface Scenario {
  installationId: number;
  login: string;
  repoIds: number[];
}

/** A fresh installation id, account and repositories, so tests never collide. */
function scenario(): Scenario {
  const suffix = randomUUID().slice(0, 8);
  return {
    installationId: randomInt(1_000_000_000, 2_000_000_000),
    login: `Acme-${suffix}`,
    repoIds: [
      randomInt(1, 2_000_000_000),
      randomInt(1, 2_000_000_000),
      randomInt(1, 2_000_000_000),
    ],
  };
}

/** Rewrites a golden payload to the scenario (repositories map positionally to its repo ids). */
function payloadOf(name: string, s: Scenario): Record<string, unknown> {
  const body = fixture(name);
  body.installation.id = s.installationId;
  body.installation.account.login = s.login;
  const repoLists = [body.repositories, body.repositories_added, body.repositories_removed];
  const originals: Record<number, number> = { 700001: 0, 700002: 1, 700003: 2 };
  for (const list of repoLists) {
    for (const repo of list ?? []) {
      const index = originals[repo.id as number]!;
      repo.id = s.repoIds[index];
      repo.full_name = `${s.login}/${repo.name}`;
    }
  }
  return body;
}

describe('installation lifecycle (integration)', () => {
  const admin = adminDb();
  const appRedis = new Redis(process.env.RG_TEST_REDIS_URL!);
  const redis = new Redis(process.env.RG_TEST_REDIS_URL!);
  const dispatched: unknown[] = [];
  const cancelled: string[][] = [];
  const verify = jest.fn(() => Promise.resolve('valid'));
  let failCancel = false;
  let app: NestExpressApplication;
  let gate: InstallationGate;
  const deliveryIds: string[] = [];
  const logins: string[] = [];
  const redisKeys: string[] = [];

  beforeAll(async () => {
    app = await createTestApp(undefined, [], {
      redis: appRedis,
      env: {
        DATABASE_URL: process.env.RG_TEST_DATABASE_URL!,
        DB_APP_ROLE: 'rg_api',
        GITHUB_ENABLED: 'true',
        GITHUB_APP_ID: '1',
        GITHUB_APP_PRIVATE_KEY: KEY.pem.replace(/\n/g, '\\n'),
        GITHUB_WEBHOOK_SECRET: SECRET,
        GITHUB_CLIENT_ID: 'c',
        GITHUB_CLIENT_SECRET: 's',
      },
      configure: (builder) =>
        builder
          .overrideProvider(GithubPermissionsMonitor)
          .useValue({ enabled: false, status: () => 'unknown', verify })
          .overrideProvider(PROVIDER_EVENT_SINK)
          .useValue({
            dispatch: (event: unknown) => {
              dispatched.push(event);
              return Promise.resolve();
            },
          })
          .overrideProvider(JOB_CANCELLER)
          .useValue({
            cancelQueuedForRepositories: (_trx: unknown, ids: string[]) => {
              if (failCancel) return Promise.reject(new Error('queue unavailable'));
              cancelled.push(ids);
              return Promise.resolve(ids.length);
            },
          }),
    });
    await app.init();
    gate = app.get<InstallationGate>(INSTALLATION_GATE);
  });

  afterAll(async () => {
    await app.close();
    const orgs = await admin
      .selectFrom('organizations')
      .select('id')
      .where('display_name', 'in', logins)
      .execute();
    if (orgs.length) {
      await admin
        .deleteFrom('organizations')
        .where(
          'id',
          'in',
          orgs.map((o) => o.id),
        )
        .execute();
    }
    if (deliveryIds.length) {
      await admin
        .deleteFrom('webhook_deliveries')
        .where('delivery_id', 'in', deliveryIds)
        .execute();
      await redis.del(...deliveryIds.map(deliveryKey));
    }
    if (redisKeys.length) await redis.del(...redisKeys);
    await admin.destroy();
    redis.disconnect();
  });

  beforeEach(() => {
    dispatched.length = 0;
    cancelled.length = 0;
    failCancel = false;
    verify.mockClear();
    resetCounterTotals();
  });

  async function send(event: 'installation' | 'installation_repositories', body: unknown) {
    const deliveryId = `it-${randomUUID()}`;
    deliveryIds.push(deliveryId);
    const raw = Buffer.from(JSON.stringify(body));
    return request(app.getHttpServer())
      .post('/api/v1/webhooks/github')
      .set('content-type', 'application/json')
      .set('x-github-event', event)
      .set('x-github-delivery', deliveryId)
      .set('x-hub-signature-256', signWebhookBody(SECRET, raw))
      .send(raw.toString());
  }

  const install = async (s: Scenario) => {
    logins.push(s.login);
    const res = await send('installation', payloadOf('installation.created.json', s));
    expect(res.status).toBe(202);
    expect(res.body.accepted).toBe(true);
  };

  const installationRow = (s: Scenario) =>
    admin
      .selectFrom('provider_installations')
      .selectAll()
      .where('provider_installation_id', '=', String(s.installationId))
      .executeTakeFirst();

  const reposOf = async (s: Scenario) =>
    admin
      .selectFrom('repositories')
      .selectAll()
      .where('provider_repo_id', 'in', s.repoIds.map(String))
      .orderBy('full_name')
      .execute();

  /** A pull request with one run in the given state, for the first repository of the scenario. */
  async function seedRun(s: Scenario, repoIndex: number, state: string, number: number) {
    const repo = (await reposOf(s)).find(
      (r) => r.provider_repo_id === String(s.repoIds[repoIndex]),
    )!;
    const sha = 'a'.repeat(40);
    const pr = await admin
      .insertInto('pull_requests')
      .values({
        organization_id: repo.organization_id,
        repository_id: repo.id,
        provider_number: number,
        title: 't',
        author_login: 'octocat',
        base_ref: 'main',
        head_ref: 'feature',
        base_sha: sha,
        head_sha: sha,
        state: 'open',
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    const run = await admin
      .insertInto('review_runs')
      .values({
        organization_id: repo.organization_id,
        repository_id: repo.id,
        pull_request_id: pr.id,
        base_sha: sha,
        head_sha: sha,
        state,
        trigger: 'webhook',
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    return { repo, runId: run.id };
  }

  const runState = async (id: string) =>
    await admin
      .selectFrom('review_runs')
      .select(['state', 'completed_at'])
      .where('id', '=', id)
      .executeTakeFirstOrThrow();

  it('installation_created_creates_org_and_repos', async () => {
    const s = scenario();
    await install(s);

    const inst = (await installationRow(s))!;
    expect(inst).toMatchObject({
      provider: 'github',
      account_login: s.login,
      account_type: 'organization',
      state: 'active',
      suspended_at: null,
      deleted_at: null,
    });
    expect(inst.permissions).toMatchObject({ pull_requests: 'write', contents: 'read' });

    const org = await admin
      .selectFrom('organizations')
      .selectAll()
      .where('id', '=', inst.organization_id)
      .executeTakeFirstOrThrow();
    expect(org.slug).toBe(s.login.toLowerCase());
    expect(org.display_name).toBe(s.login);

    const repos = await reposOf(s);
    expect(repos.map((r) => [r.full_name, r.visibility, r.enabled, r.access_state])).toEqual([
      [`${s.login}/billing`, 'private', true, 'active'],
      [`${s.login}/web`, 'public', true, 'active'],
    ]);
    expect(repos.every((r) => r.organization_id === org.id && r.installation_id === inst.id)).toBe(
      true,
    );

    // The delivery row learned its organization and ended processed.
    const delivery = await admin
      .selectFrom('webhook_deliveries')
      .select(['status', 'organization_id', 'provider_installation_id'])
      .where('delivery_id', '=', deliveryIds.at(-1)!)
      .executeTakeFirstOrThrow();
    expect(delivery).toEqual({
      status: 'processed',
      organization_id: org.id,
      provider_installation_id: String(s.installationId),
    });
    expect(dispatched).toEqual([]);
    expect(counterTotal('installation_events_total', { action: 'created' })).toBe(1);
  });

  it('is idempotent across distinct deliveries and reuses the organization on reinstall', async () => {
    const s = scenario();
    await install(s);
    await install(s);
    expect(await reposOf(s)).toHaveLength(2);

    // A reinstall gets a new installation id for the same account.
    const again = { ...s, installationId: randomInt(1_000_000_000, 2_000_000_000) };
    await send('installation', payloadOf('installation.created.json', again));
    const first = (await installationRow(s))!;
    const second = (await installationRow(again))!;
    expect(second.organization_id).toBe(first.organization_id);
    // The repositories moved to the new installation row, not duplicated.
    const repos = await reposOf(s);
    expect(repos).toHaveLength(2);
    expect(repos.every((r) => r.installation_id === second.id)).toBe(true);
  });

  it('installation_deleted_cancels_jobs_and_purges_tokens', async () => {
    const s = scenario();
    await install(s);
    const active = await seedRun(s, 0, 'INDEXING', 1);
    const done = await seedRun(s, 1, 'COMPLETED', 1);
    const tokenKey = `rg:gh:itok:${s.installationId}:0123456789abcdef`;
    const otherKey = `rg:gh:itok:${s.installationId + 1}:0123456789abcdef`;
    const lockKey = `rg:gh:itok-lock:${s.installationId}`;
    redisKeys.push(tokenKey, otherKey, lockKey);
    await redis.set(tokenKey, 'ciphertext', 'EX', 600);
    await redis.set(otherKey, 'ciphertext', 'EX', 600);
    await redis.set(lockKey, '1', 'EX', 600);
    await expect(gate.assertUsable(String(s.installationId))).resolves.toBeUndefined();

    const res = await send('installation', payloadOf('installation.deleted.json', s));
    expect(res.status).toBe(202);
    expect(res.body.accepted).toBe(true);

    expect(await installationRow(s)).toMatchObject({ state: 'deleted' });
    expect((await installationRow(s))!.deleted_at).not.toBeNull();
    const repos = await reposOf(s);
    expect(repos.map((r) => [r.enabled, r.access_state])).toEqual([
      [false, 'installation_deleted'],
      [false, 'installation_deleted'],
    ]);
    // Queued jobs were cancelled for both repositories, the running review was cancelled by CAS,
    // and a finished review was left alone.
    expect(cancelled).toHaveLength(1);
    expect([...cancelled[0]!].sort()).toEqual([active.repo.id, done.repo.id].sort());
    expect(await runState(active.runId)).toMatchObject({ state: 'CANCELLED' });
    expect((await runState(active.runId)).completed_at).not.toBeNull();
    expect((await runState(done.runId)).state).toBe('COMPLETED');
    // Cached tokens of this installation are gone, those of others are not.
    expect(await redis.exists(tokenKey)).toBe(0);
    expect(await redis.exists(lockKey)).toBe(0);
    expect(await redis.exists(otherKey)).toBe(1);
    // After deletion nothing may mint credentials or claim work: the broker answers 409.
    await expect(gate.assertUsable(String(s.installationId))).rejects.toMatchObject({
      name: 'InstallationInactiveError',
      state: 'deleted',
    });

    // Replaying the deletion (a different delivery) is a harmless no-op.
    const replay = await send('installation', payloadOf('installation.deleted.json', s));
    expect(replay.body.accepted).toBe(true);
    expect(await installationRow(s)).toMatchObject({ state: 'deleted' });
  });

  it('a deletion of an unknown installation is ignored', async () => {
    const s = scenario();
    const res = await send('installation', payloadOf('installation.deleted.json', s));
    expect(res.status).toBe(202);
    expect(res.body).toMatchObject({ accepted: false, reason: 'unknown_installation' });
    expect(await installationRow(s)).toBeUndefined();
    const delivery = await admin
      .selectFrom('webhook_deliveries')
      .select(['status', 'organization_id'])
      .where('delivery_id', '=', deliveryIds.at(-1)!)
      .executeTakeFirstOrThrow();
    expect(delivery).toEqual({ status: 'ignored', organization_id: null });
  });

  it('suspended_installation_refuses_clone_credentials', async () => {
    const s = scenario();
    await install(s);
    const tokenKey = `rg:gh:itok:${s.installationId}:fedcba9876543210`;
    redisKeys.push(tokenKey);
    await redis.set(tokenKey, 'ciphertext', 'EX', 600);

    await send('installation', payloadOf('installation.suspend.json', s)).then((r) =>
      expect(r.body.accepted).toBe(true),
    );
    const suspended = (await installationRow(s))!;
    expect(suspended.state).toBe('suspended');
    expect(suspended.suspended_at).not.toBeNull();
    // Suspension pauses processing; it does not disable repositories.
    expect((await reposOf(s)).every((r) => r.enabled)).toBe(true);
    expect(await redis.exists(tokenKey)).toBe(0);
    const refusal = await gate.assertUsable(String(s.installationId)).catch((e: unknown) => e);
    expect(refusal).toBeInstanceOf(InstallationInactiveError);
    expect((refusal as InstallationInactiveError).state).toBe('suspended');

    await send('installation', payloadOf('installation.unsuspend.json', s)).then((r) =>
      expect(r.body.accepted).toBe(true),
    );
    expect(await installationRow(s)).toMatchObject({ state: 'active', suspended_at: null });
    await expect(gate.assertUsable(String(s.installationId))).resolves.toBeUndefined();
    await expect(gate.assertUsable('1')).rejects.toMatchObject({ state: 'unknown' });
  });

  it('repositories_removed_disables', async () => {
    const s = scenario();
    await install(s);
    const running = await seedRun(s, 0, 'REVIEWING', 1);

    const removed = await send(
      'installation_repositories',
      payloadOf('installation_repositories.removed.json', s),
    );
    expect(removed.body.accepted).toBe(true);
    const billing = (await reposOf(s)).find((r) => r.full_name.endsWith('/billing'))!;
    const web = (await reposOf(s)).find((r) => r.full_name.endsWith('/web'))!;
    expect([billing.enabled, billing.access_state]).toEqual([false, 'removed']);
    expect([web.enabled, web.access_state]).toEqual([true, 'active']);
    expect(cancelled).toEqual([[running.repo.id]]);
    expect((await runState(running.runId)).state).toBe('CANCELLED');

    // Access granted again: the repository is re-enabled and a new one is created.
    const added = await send(
      'installation_repositories',
      payloadOf('installation_repositories.added.json', s),
    );
    expect(added.body.accepted).toBe(true);
    const created = (await reposOf(s)).find((r) => r.full_name.endsWith('/infra'))!;
    expect([created.enabled, created.access_state, created.visibility]).toEqual([
      true,
      'active',
      'private',
    ]);
    expect(counterTotal('installation_events_total', { action: 'repositories_removed' })).toBe(1);
    expect(counterTotal('installation_events_total', { action: 'repositories_added' })).toBe(1);

    // Re-adding the removed repository re-enables it.
    const readd = payloadOf('installation_repositories.added.json', s);
    (readd.repositories_added as { id: number; full_name: string; name: string }[])[0] = {
      id: s.repoIds[0]!,
      name: 'billing',
      full_name: `${s.login}/billing`,
      private: true,
    } as never;
    await send('installation_repositories', readd);
    const back = (await reposOf(s)).find((r) => r.full_name.endsWith('/billing'))!;
    expect([back.enabled, back.access_state]).toEqual([true, 'active']);
  });

  it('repositories_added creates the installation when the created event was missed', async () => {
    const s = scenario();
    logins.push(s.login);
    const res = await send(
      'installation_repositories',
      payloadOf('installation_repositories.added.json', s),
    );
    expect(res.body.accepted).toBe(true);
    expect(await installationRow(s)).toMatchObject({ state: 'active' });
    expect((await reposOf(s)).map((r) => r.full_name)).toEqual([`${s.login}/infra`]);
  });

  it('new_permissions_rechecked', async () => {
    const s = scenario();
    await install(s);
    expect(verify).not.toHaveBeenCalled();
    const res = await send(
      'installation',
      payloadOf('installation.new_permissions_accepted.json', s),
    );
    expect(res.body.accepted).toBe(true);
    expect((await installationRow(s))!.permissions).toMatchObject({ statuses: 'read' });
    expect(await installationRow(s)).toMatchObject({ state: 'active' });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(verify).toHaveBeenCalledTimes(1);
  });

  it('rolls the delivery back when handling fails, so the retry applies it', async () => {
    const s = scenario();
    await install(s);
    failCancel = true;
    const deliveryId = `it-${randomUUID()}`;
    deliveryIds.push(deliveryId);
    const raw = Buffer.from(JSON.stringify(payloadOf('installation.deleted.json', s)));
    const post = () =>
      request(app.getHttpServer())
        .post('/api/v1/webhooks/github')
        .set('content-type', 'application/json')
        .set('x-github-event', 'installation')
        .set('x-github-delivery', deliveryId)
        .set('x-hub-signature-256', signWebhookBody(SECRET, raw))
        .send(raw.toString());

    expect((await post()).status).toBe(503);
    expect(await installationRow(s)).toMatchObject({ state: 'active' });
    expect((await reposOf(s)).every((r) => r.enabled)).toBe(true);
    expect(
      await admin
        .selectFrom('webhook_deliveries')
        .select('id')
        .where('delivery_id', '=', deliveryId)
        .execute(),
    ).toHaveLength(0);

    failCancel = false;
    const retry = await post();
    expect(retry.status).toBe(202);
    expect(retry.body.accepted).toBe(true);
    expect(await installationRow(s)).toMatchObject({ state: 'deleted' });
  });
});
