import { Controller, Get, type INestApplication } from '@nestjs/common';
import { createServer, type Server, type Socket } from 'node:net';
import { ESLint } from 'eslint';
import { sql } from 'kysely';
import request from 'supertest';
import { DbService, PG_POOL } from '../../src/db/db.module';
import { isDbUnavailable } from '../../src/db/errors';
import { createKysely, createPool } from '../../src/db/kysely.provider';
import { createTestApp } from '../helpers';

@Controller('db-probe')
class DbProbeController {
  constructor(private readonly dbs: DbService) {}

  @Get()
  async probe(): Promise<{ ok: true }> {
    await this.dbs.db.selectFrom('organizations').select('id').limit(1).execute();
    return { ok: true };
  }
}

describe('db module', () => {
  it('generated_types_compile_against_queries', () => {
    // Compiles only if the generated DB type matches these column names (typecheck gate).
    const db = createKysely(createPool({ connectionString: 'postgres://u:p@127.0.0.1:1/db' }));
    const query = db
      .selectFrom('pull_requests as pr')
      .innerJoin('repositories as r', (join) =>
        join
          .onRef('r.id', '=', 'pr.repository_id')
          .onRef('r.organization_id', '=', 'pr.organization_id'),
      )
      .select(['pr.id', 'pr.head_sha', 'r.full_name'])
      .where('pr.state', '=', 'open')
      .compile();
    expect(query.sql).toContain('"pr"."head_sha"');
    expect(query.parameters).toEqual(['open']);
    return db.destroy();
  });

  describe('pool_acquire_timeout_returns_503', () => {
    let blackhole: Server;
    const sockets: Socket[] = [];
    let app: INestApplication;

    beforeAll(async () => {
      // Accepts TCP connections and never speaks: connecting to postgres here stalls forever.
      blackhole = createServer((socket) => sockets.push(socket));
      await new Promise<void>((resolve) => blackhole.listen(0, '127.0.0.1', resolve));
      const { port } = blackhole.address() as { port: number };
      const pool = createPool({
        connectionString: `postgres://u:p@127.0.0.1:${port}/db`,
        connectionTimeoutMillis: 200,
      });
      app = await createTestApp(undefined, [DbProbeController], {
        configure: (builder) => builder.overrideProvider(PG_POOL).useValue(pool),
      });
      await app.init();
    });

    afterAll(async () => {
      await app.close();
      sockets.forEach((s) => s.destroy());
      await new Promise<void>((resolve) => blackhole.close(() => resolve()));
    });

    it('answers 503 with Retry-After', async () => {
      const res = await request(app.getHttpServer()).get('/api/v1/db-probe');
      expect(res.status).toBe(503);
      expect(res.headers['retry-after']).toBe('1');
      expect(res.headers['content-type']).toContain('application/problem+json');
    });
  });

  it('maps statement timeouts and connection errors to unavailable', () => {
    expect(
      isDbUnavailable(Object.assign(new Error('canceling statement'), { code: '57014' })),
    ).toBe(true);
    expect(isDbUnavailable(Object.assign(new Error('x'), { code: 'ECONNREFUSED' }))).toBe(true);
    expect(isDbUnavailable(new Error('timeout exceeded when trying to connect'))).toBe(true);
    expect(isDbUnavailable(new Error('boom'))).toBe(false);
    expect(isDbUnavailable(Object.assign(new Error('dup'), { code: '23505' }))).toBe(false);
  });

  it('sql_raw_lint_rule_enforced', async () => {
    const eslint = new ESLint({ cwd: process.cwd() });
    const code = "import { sql } from 'kysely';\nexport const q = sql.raw('select 1');\n";
    const outside = await eslint.lintText(code, { filePath: 'src/orders/orders.service.ts' });
    expect(outside[0]?.messages.map((m) => m.ruleId)).toContain('no-restricted-syntax');
    const inside = await eslint.lintText(code, { filePath: 'src/db/raw-helper.ts' });
    expect(inside[0]?.messages.map((m) => m.ruleId)).not.toContain('no-restricted-syntax');
  });

  it('keeps sql tagged templates parameterized', () => {
    const compiled = sql`select ${'x'}`.compile(
      createKysely(createPool({ connectionString: 'postgres://u:p@127.0.0.1:1/db' })),
    );
    expect(compiled.sql).toBe('select $1');
  });
});
