import { context, trace } from '@opentelemetry/api';
import { AsyncLocalStorageContextManager } from '@opentelemetry/context-async-hooks';
import {
  BasicTracerProvider,
  InMemorySpanExporter,
  SimpleSpanProcessor,
} from '@opentelemetry/sdk-trace-base';
import { parseEnv } from '../../src/config/env.schema';
import { parseOtlpHeaders, parseTelemetryConfig } from '../../src/telemetry/config';
import { createJsonLogger } from '../../src/telemetry/json-logger';
import { getTelemetryHandle, startTelemetry } from '../../src/telemetry/sdk';
import { TracerService } from '../../src/telemetry/tracer.service';
import { VALID_ENV } from '../helpers';

describe('telemetry config and no-op mode', () => {
  it('sdk_boots_without_endpoint_as_noop', async () => {
    const config = parseTelemetryConfig({});
    expect(config.enabled).toBe(false);

    const handle = startTelemetry(config);
    expect(handle.enabled).toBe(false);
    expect(getTelemetryHandle()).toBe(handle);
    // A second start (hot reload) returns the same handle instead of re-registering.
    expect(startTelemetry(config)).toBe(handle);

    const span = trace.getTracer('t').startSpan('x');
    expect(span.isRecording()).toBe(false);
    span.end();
    await expect(handle.shutdown()).resolves.toBeUndefined();
  });

  it('is_disabled_when_flag_false_even_with_endpoint', () => {
    const config = parseTelemetryConfig({
      OTEL_EXPORTER_OTLP_ENDPOINT: 'http://127.0.0.1:25080/api/default/',
      RG_OTEL_ENABLED: 'false',
    });
    expect(config.enabled).toBe(false);
    const on = parseTelemetryConfig({
      OTEL_EXPORTER_OTLP_ENDPOINT: 'http://127.0.0.1:25080/api/default/',
    });
    expect(on.enabled).toBe(true);
    expect(on.endpoint).toBe('http://127.0.0.1:25080/api/default');
    expect(on.serviceName).toBe('reviewgraph-api');
  });

  it('parses_otlp_headers', () => {
    expect(parseOtlpHeaders('Authorization=Basic%20YWJj, x-org=default')).toEqual({
      Authorization: 'Basic YWJj',
      'x-org': 'default',
    });
  });

  it('malformed_otlp_headers_fail_boot_without_echoing_values', () => {
    const secret = 'hunter2-secret';
    expect(() => parseOtlpHeaders(secret)).toThrow();
    let message = '';
    try {
      parseEnv({ ...VALID_ENV, OTEL_EXPORTER_OTLP_HEADERS: secret });
    } catch (err) {
      message = (err as Error).message;
    }
    expect(message).toContain('OTEL_EXPORTER_OTLP_HEADERS');
    expect(message).not.toContain(secret);
  });
});

describe('tracing helpers', () => {
  const exporter = new InMemorySpanExporter();
  const provider = new BasicTracerProvider({
    spanProcessors: [new SimpleSpanProcessor(exporter)],
  });

  beforeAll(() => {
    context.setGlobalContextManager(new AsyncLocalStorageContextManager().enable());
    trace.setGlobalTracerProvider(provider);
  });
  afterAll(async () => {
    trace.disable();
    context.disable();
    await provider.shutdown();
  });
  beforeEach(() => exporter.reset());

  it('with_span_records_exception_and_ends', async () => {
    const tracer = new TracerService();
    const boom = new Error('boom');
    await expect(
      tracer.withSpan('publication', { review_run_id: 'run-1' }, () => {
        throw boom;
      }),
    ).rejects.toBe(boom);

    const [span] = exporter.getFinishedSpans();
    expect(span?.name).toBe('publication');
    expect(span?.attributes['review_run_id']).toBe('run-1');
    expect(span?.status.code).toBe(2); // ERROR
    expect(span?.events.some((e) => e.name === 'exception')).toBe(true);
  });

  it('with_span_nests_under_the_active_span', async () => {
    const tracer = new TracerService();
    await tracer.withSpan('reviewer_execution', {}, () =>
      tracer.withSpan('model_request', {}, () => 'ok'),
    );
    const spans = exporter.getFinishedSpans();
    const parent = spans.find((s) => s.name === 'reviewer_execution');
    const child = spans.find((s) => s.name === 'model_request');
    expect(child?.parentSpanContext?.spanId).toBe(parent?.spanContext().spanId);
  });

  it('logger_attaches_trace_and_span_ids', async () => {
    const lines: string[] = [];
    const logger = createJsonLogger('info', { write: (chunk: string) => void lines.push(chunk) });
    const tracer = new TracerService();

    logger.info({ context: 'Test' }, 'outside');
    let expected: { traceId: string; spanId: string } | undefined;
    await tracer.withSpan('diff_analysis', { review_run_id: 'run-9' }, (span) => {
      expected = span.spanContext();
      logger.info({ context: 'Test' }, 'inside');
    });

    const [outside, inside] = lines.map((l) => JSON.parse(l) as Record<string, unknown>);
    expect(outside?.trace_id).toBeUndefined();
    expect(inside).toMatchObject({
      level: 'info',
      context: 'Test',
      message: 'inside',
      trace_id: expected?.traceId,
      span_id: expected?.spanId,
      review_run_id: 'run-9',
    });
    expect(typeof inside?.timestamp).toBe('string');
  });

  it('logger_redacts_credential_headers', () => {
    const lines: string[] = [];
    const logger = createJsonLogger('info', { write: (chunk: string) => void lines.push(chunk) });
    logger.info({ headers: { authorization: 'Bearer abc' } }, 'req');
    expect(lines[0]).not.toContain('Bearer abc');
  });
});
