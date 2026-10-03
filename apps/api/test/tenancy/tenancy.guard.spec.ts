import type { NestExpressApplication } from '@nestjs/platform-express';
import request from 'supertest';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import { MembershipService } from '../../src/tenancy/membership.service';
import type { MembershipRole } from '../../src/tenancy/request-context';
import { satisfies } from '../../src/tenancy/roles.decorator';
import { createTestApp } from '../helpers';
import { installTestAuth, TenancyProbeController } from '../helpers/tenancy-probe';

const USER = '0190f3a2-0000-7000-8000-0000000000aa';
const OWN_REPO = '0190f3a2-0000-7000-8000-0000000000b1';
const FOREIGN_REPO = '0190f3a2-0000-7000-8000-0000000000b2';
const ORG = '0190f3a2-0000-7000-8000-0000000000c1';

describe('TenancyGuard (unit, fake lookups)', () => {
  let app: NestExpressApplication;
  let role: MembershipRole | null = 'viewer';

  beforeAll(async () => {
    app = await createTestApp(undefined, [TenancyProbeController], {
      configure: (builder) =>
        builder.overrideProvider(MembershipService).useValue({
          resolveOrg: (_kind: string, id: string) =>
            Promise.resolve(id === OWN_REPO ? ORG : id === FOREIGN_REPO ? 'other-org' : null),
          roleOf: (_user: string, org: string) => Promise.resolve(org === ORG ? role : null),
          listForUser: () => Promise.resolve([{ organizationId: ORG, role: 'viewer' }]),
        }),
    });
    installTestAuth(app);
    await app.init();
  });

  afterAll(() => app.close());
  beforeEach(() => {
    role = 'viewer';
    resetCounterTotals();
  });

  const get = (path: string, user: string | undefined = USER) => {
    const req = request(app.getHttpServer()).get(`/api/v1/probe/repos${path}`);
    return user ? req.set('x-test-user', user) : req;
  };

  it('guard_404_for_foreign_repo', async () => {
    await get(`/${FOREIGN_REPO}`).expect(404);
    expect(counterTotal('tenancy_denied_total', { reason: 'not_member' })).toBe(1);
  });

  it('answers 404 for an unknown or malformed id, never 403', async () => {
    await get('/0190f3a2-0000-7000-8000-0000000000ff').expect(404);
    await get('/not-a-uuid').expect(404);
  });

  it('guard_role_maintainer_required', async () => {
    const patch = () =>
      request(app.getHttpServer())
        .patch(`/api/v1/probe/repos/${OWN_REPO}`)
        .set('x-test-user', USER);
    await patch().expect(403);
    expect(counterTotal('tenancy_denied_total', { reason: 'role' })).toBe(1);
    role = 'member';
    await patch().expect(200);
    role = 'admin';
    await patch().expect(200);
  });

  it('stores the tenant for the handler', async () => {
    const res = await get(`/${OWN_REPO}`).expect(200);
    expect(res.body).toEqual({ organizationId: ORG, role: 'viewer' });
  });

  it('falls back to the only membership of the caller when no resource is named', async () => {
    const res = await get('').expect(200);
    expect(res.body.organizationId).toBe(ORG);
  });

  it('requires a user', async () => {
    await get(`/${OWN_REPO}`, '').expect(401);
  });

  it('orders roles viewer < maintainer < admin', () => {
    expect(satisfies('viewer', 'maintainer')).toBe(false);
    expect(satisfies('member', 'maintainer')).toBe(true);
    expect(satisfies('member', 'admin')).toBe(false);
    expect(satisfies('owner', 'admin')).toBe(true);
  });
});
