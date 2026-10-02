import { execFile } from 'node:child_process';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { PARENT_SPAN_ID, TRACE_ID } from './constants';

interface SpanRecord {
  name: string;
  kind: number;
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  attributes: Record<string, unknown>;
  instrumentationScope: string;
}
interface ScenarioResult {
  tracedStatus: number;
  healthStatus: number;
  spans: SpanRecord[];
}

const SPAN_KIND_SERVER = 1;

/**
 * Auto-instrumentation relies on Node module hooks that Jest's sandboxed `require` bypasses,
 * so the scenario runs in a child process (see run.cjs) and the spans are asserted here.
 */
describe('telemetry instrumentation (child process)', () => {
  let result: ScenarioResult;

  beforeAll(async () => {
    const { stdout } = await promisify(execFile)(process.execPath, [join(__dirname, 'run.cjs')], {
      timeout: 60_000,
      cwd: join(__dirname, '..', '..'),
    });
    const line = stdout.split('\n').find((l) => l.startsWith('RESULT:'));
    if (!line) throw new Error(`scenario produced no result:\n${stdout}`);
    result = JSON.parse(line.slice('RESULT:'.length)) as ScenarioResult;
  }, 90_000);

  const server = () => result.spans.find((s) => s.kind === SPAN_KIND_SERVER);

  it('http_request_creates_server_span_with_route', () => {
    expect(result.tracedStatus).toBe(200);
    const span = server();
    expect(span?.name).toBe('HTTP GET /api/v1/things/:id');
    expect(span?.attributes['http.route']).toBe('/api/v1/things/:id');
    expect(span?.attributes['http.response.status_code']).toBe(200);
    expect(span?.attributes['request_id']).toEqual(expect.any(String));
  });

  it('incoming_traceparent_becomes_parent', () => {
    const span = server();
    expect(span?.traceId).toBe(TRACE_ID);
    expect(span?.parentSpanId).toBe(PARENT_SPAN_ID);
    // Child spans (stage, pg, redis) share the trace.
    expect(result.spans.every((s) => s.traceId === TRACE_ID)).toBe(true);
  });

  it('pg_query_span_has_no_parameter_values', () => {
    const spans = result.spans.filter((s) => s.instrumentationScope.includes('instrumentation-pg'));
    expect(spans.some((s) => s.name.startsWith('pg.query'))).toBe(true);
    expect(JSON.stringify(spans)).not.toContain('super-secret-param');
  });

  it('ioredis_command_span_created', () => {
    const spans = result.spans.filter((s) =>
      s.instrumentationScope.includes('instrumentation-ioredis'),
    );
    expect(spans.some((s) => s.name === 'set')).toBe(true);
    // Only the command name is recorded, never keys or values.
    const serialized = JSON.stringify(spans);
    expect(serialized).not.toContain('some-key');
    expect(serialized).not.toContain('super-secret-value');
  });

  it('health_route_not_traced', () => {
    expect(result.healthStatus).toBe(200);
    expect(result.spans.filter((s) => s.kind === SPAN_KIND_SERVER)).toHaveLength(1);
    expect(JSON.stringify(result.spans)).not.toContain('health');
  });

  it('http_span_records_no_query_string_or_auth_header', () => {
    const serialized = JSON.stringify(server()?.attributes);
    expect(serialized).not.toContain('hunter2');
    expect(serialized).not.toContain('top-secret-token');
    expect(serialized).not.toContain('?');
  });

  it('stage_span_created_by_tracer_service', () => {
    const stage = result.spans.find((s) => s.name === 'diff_analysis');
    expect(stage?.attributes['request_id']).toBe('r-42');
    expect(stage?.parentSpanId).toBe(server()?.spanId);
  });
});
