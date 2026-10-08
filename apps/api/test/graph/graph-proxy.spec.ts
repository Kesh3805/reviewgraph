import { createServer, type IncomingMessage, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import type { NestExpressApplication } from '@nestjs/platform-express';
import RedisMock from 'ioredis-mock';
import request from 'supertest';
import { SessionService } from '../../src/auth/session.service';
import { counterTotal } from '../../src/common/metrics';
import { GRAPH_SCOPE, type GraphScope } from '../../src/graph/graph-scope';
import { redactExcerpt } from '../../src/graph/redact';
import { ServiceKeyRing, verifyServiceToken } from '../../src/internal/service-token';
import { MembershipService } from '../../src/tenancy/membership.service';
import { createTestApp } from '../helpers';
import { asUser, CSRF_HEADERS, testSessions } from '../helpers/tenancy-probe';

const ORG = '0190f3a2-0000-7000-8000-00000000000a';
const REPO = '0190f3a2-0000-7000-8000-00000000000b';
const FOREIGN_REPO = '0190f3a2-0000-7000-8000-00000000000c';
const RUN = '0190f3a2-0000-7000-8000-00000000000d';
const SNAP = '0190f3a2-0000-7000-8000-0000000000aa';
const OTHER_SNAP = '0190f3a2-0000-7000-8000-0000000000bb';
const USER = '0190f3a2-0000-7000-8000-0000000000ff';
const SECRET = 's'.repeat(32);

interface Captured {
  method: string;
  url: string;
  authorization: string | undefined;
  body: string;
}

/** A stand-in review engine (API-013 contract) that records every request. */
function fakeEngine(): Promise<{
  server: Server;
  url: string;
  requests: Captured[];
  fail: { status: number };
}> {
  const requests: Captured[] = [];
  const fail = { status: 0 };
  const server = createServer((req: IncomingMessage, res) => {
    let body = '';
    req.on('data', (c: Buffer) => (body += c.toString()));
    req.on('end', () => {
      requests.push({
        method: req.method ?? '',
        url: req.url ?? '',
        authorization: req.headers.authorization,
        body,
      });
      if (fail.status) {
        res.writeHead(fail.status).end();
        return;
      }
      const url = new URL(req.url ?? '/', 'http://engine');
      res.setHeader('content-type', 'application/json');
      if (url.pathname.endsWith('/source')) {
        res.end(
          JSON.stringify({
            snapshot_id: SNAP,
            path: url.searchParams.get('path'),
            start: Number(url.searchParams.get('start')),
            end: Number(url.searchParams.get('end')),
            // The engine "forgot" to redact: the proxy must still redact.
            text: 'const KEY = "sk-live-abcdefghijklmnopqrstuvwxyz"\nexport const answer = 42;',
          }),
        );
        return;
      }
      if (url.pathname.endsWith('/subgraph')) {
        res.end(JSON.stringify({ snapshot_id: SNAP, truncated: true, nodes: [], edges: [] }));
        return;
      }
      res.end(JSON.stringify({ snapshot_id: SNAP, truncated: false, items: [{ key: 'k1' }] }));
    });
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as AddressInfo;
      resolve({ server, url: `http://127.0.0.1:${port}`, requests, fail });
    });
  });
}

describe('graph proxy (API-011)', () => {
  let app: NestExpressApplication;
  let engine: Awaited<ReturnType<typeof fakeEngine>>;
  const audits: unknown[] = [];

  const scope: GraphScope = {
    defaultSnapshot: (_org, repo) => Promise.resolve(repo === REPO ? SNAP : null),
    snapshotBelongsTo: (_org, repo, snap) => Promise.resolve(repo === REPO && snap === SNAP),
    reviewRepository: (_org, run) => Promise.resolve(run === RUN ? REPO : null),
    recordSourceAccess: (event) => {
      audits.push(event);
      return Promise.resolve();
    },
  };

  /** Every repository and run resolves to ORG, where USER is a viewer. */
  const memberships = {
    resolveOrg: () => Promise.resolve(ORG),
    roleOf: () => Promise.resolve('viewer'),
    listForUser: () => Promise.resolve([]),
    installationOrg: () => Promise.resolve(null),
  };

  beforeAll(async () => {
    engine = await fakeEngine();
    app = await createTestApp(undefined, [], {
      redis: new RedisMock(),
      env: { ENGINE_INTERNAL_URL: engine.url, SERVICE_JWT_SECRET: SECRET },
      configure: (b) =>
        b
          .overrideProvider(SessionService)
          .useValue(testSessions)
          .overrideProvider(MembershipService)
          .useValue(memberships)
          .overrideProvider(GRAPH_SCOPE)
          .useValue(scope),
    });
    await app.init();
  });

  afterAll(async () => {
    await app.close();
    await new Promise((r) => engine.server.close(r));
  });

  beforeEach(() => {
    engine.requests.length = 0;
    engine.fail.status = 0;
  });

  const get = (path: string) =>
    request(app.getHttpServer()).get(`/api/v1${path}`).set('cookie', asUser(USER));
  const post = (path: string, body: object) =>
    request(app.getHttpServer())
      .post(`/api/v1${path}`)
      .set('cookie', asUser(USER))
      .set(CSRF_HEADERS)
      .send(body);

  it('subgraph_max_nodes_enforced', async () => {
    await post(`/repositories/${REPO}/graph/subgraph`, { seeds: ['k1'], max_nodes: 501 }).expect(
      400,
    );
    await post(`/repositories/${REPO}/graph/subgraph`, { seeds: ['k1'], depth: 4 }).expect(400);
    expect(engine.requests).toHaveLength(0);
    const ok = await post(`/repositories/${REPO}/graph/subgraph`, {
      seeds: ['k1'],
      max_nodes: 500,
    }).expect(200);
    // Budget truncation passes through.
    expect(ok.body.truncated).toBe(true);
    expect(JSON.parse(engine.requests[0]!.body)).toMatchObject({ max_nodes: 500, depth: 1 });
  });

  it('foreign_snapshot_rejected', async () => {
    await get(`/repositories/${REPO}/graph/symbols?q=User&snapshot=${OTHER_SNAP}`).expect(404);
    expect(engine.requests).toHaveLength(0);
    await get(`/repositories/${REPO}/graph/symbols?q=User&snapshot=${SNAP}`).expect(200);
    expect(engine.requests[0]!.url).toContain(
      `/internal/v1/repos/${REPO}/snapshots/${SNAP}/symbols`,
    );
    // No snapshot yet for a repository: 404, not an engine call.
    await get(`/repositories/${FOREIGN_REPO}/graph/symbols?q=User`).expect(404);
    expect(engine.requests).toHaveLength(1);
  });

  it('tenant_claims_added_server_side', async () => {
    // A client-supplied org or repo is ignored: the claims come from the route and the tenant.
    await get(
      `/repositories/${REPO}/graph/symbols/k1/neighbors?dir=in&kinds=CALLS,IMPORTS&org=${FOREIGN_REPO}&repo=${FOREIGN_REPO}`,
    ).expect(200);
    const captured = engine.requests[0]!;
    expect(captured.url).not.toContain(FOREIGN_REPO);
    expect(captured.url).toContain('dir=in');
    expect(captured.url).toContain('kinds=CALLS%2CIMPORTS');
    const token = captured.authorization!.replace(/^Bearer /, '');
    const ring = ServiceKeyRing.from([{ kid: 'default', secret: Buffer.from(SECRET, 'utf8') }]);
    const claims = await verifyServiceToken(ring, token, { audience: 'rg-engine' });
    expect(claims).toMatchObject({ iss: 'rg-api', org: ORG, repo: REPO, scope: ['graph:read'] });

    // Impact is scoped to the run's repository.
    await get(`/reviews/${RUN}/impact/k1`).expect(200);
    const impact = engine.requests[1]!;
    expect(impact.url).toBe(`/internal/v1/reviews/${RUN}/impact/k1`);
    const impactClaims = await verifyServiceToken(
      ring,
      impact.authorization!.replace(/^Bearer /, ''),
      { audience: 'rg-engine' },
    );
    expect(impactClaims.repo).toBe(REPO);
  });

  it('excerpt_redacted', async () => {
    const res = await get(`/repositories/${REPO}/source?path=src/config.ts&start=1&end=2`).expect(
      200,
    );
    expect(res.body.text).toContain('KEY="<redacted>"');
    expect(res.body.text).not.toContain('sk-live');
    expect(res.body.text).toContain('export const answer = 42;');
    expect(res.body.redacted).toBe(true);
    expect(audits).toContainEqual(
      expect.objectContaining({
        userId: USER,
        repositoryId: REPO,
        path: 'src/config.ts',
        start: 1,
        end: 2,
      }),
    );
    // At most 200 lines, repository-relative paths only.
    await get(`/repositories/${REPO}/source?path=src/a.ts&start=1&end=201`).expect(400);
    await get(`/repositories/${REPO}/source?path=../etc/passwd&start=1&end=2`).expect(400);
  });

  it('cache_hit_on_repeat', async () => {
    const path = `/repositories/${REPO}/graph/path?from=a&to=b&max_depth=3`;
    const before = counterTotal('graph_queries_total', { route: 'path', cached: 'true' });
    const first = await get(path).expect(200);
    const second = await get(path).expect(200);
    expect(second.body).toEqual(first.body);
    expect(engine.requests).toHaveLength(1);
    expect(counterTotal('graph_queries_total', { route: 'path', cached: 'true' })).toBe(before + 1);
    await get(`/repositories/${REPO}/graph/path?from=a&to=b&max_depth=7`).expect(400);
  });

  it('engine down answers 503', async () => {
    engine.fail.status = 500;
    const res = await get(`/repositories/${REPO}/graph/symbols/unknown-key`).expect(503);
    expect(res.body.status).toBe(503);
    engine.fail.status = 404;
    await get(`/repositories/${REPO}/graph/symbols/missing`).expect(404);
  });
});

describe('redactExcerpt', () => {
  it('redacts assignments of sensitive names and known token shapes, keeping structure', () => {
    const { text, redactions } = redactExcerpt(
      [
        'API_KEY=abc123',
        "const password = 'hunter2';",
        'if (password === input) {}',
        'token: ghp_0123456789abcdefghijklmnopqrstuvwxyzAB',
        'Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345',
        'const user = "alice";',
      ].join('\n'),
    );
    expect(text).toContain('API_KEY="<redacted>"');
    expect(text).toContain('password="<redacted>";');
    expect(text).toContain('if (password === input) {}');
    expect(text).not.toContain('ghp_');
    expect(text).not.toContain('abcdefghijklmnopqrstuvwxyz012345');
    expect(text).toContain('const user = "alice";');
    expect(redactions).toBeGreaterThanOrEqual(4);
  });
});
