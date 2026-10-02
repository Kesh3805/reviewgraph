import { metrics } from '@opentelemetry/api';
import { ExportResultCode, type ExportResult } from '@opentelemetry/core';
import { OTLPLogExporter } from '@opentelemetry/exporter-logs-otlp-proto';
import { OTLPMetricExporter } from '@opentelemetry/exporter-metrics-otlp-proto';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-proto';
import { HttpInstrumentation } from '@opentelemetry/instrumentation-http';
import { IORedisInstrumentation } from '@opentelemetry/instrumentation-ioredis';
import { PgInstrumentation } from '@opentelemetry/instrumentation-pg';
import { resourceFromAttributes } from '@opentelemetry/resources';
import { BatchLogRecordProcessor, type LogRecordProcessor } from '@opentelemetry/sdk-logs';
import { PeriodicExportingMetricReader, type IMetricReader } from '@opentelemetry/sdk-metrics';
import { NodeSDK } from '@opentelemetry/sdk-node';
import { BatchSpanProcessor, type SpanProcessor } from '@opentelemetry/sdk-trace-base';
import { ATTR_SERVICE_NAME, ATTR_SERVICE_VERSION } from '@opentelemetry/semantic-conventions';
import type { IncomingMessage } from 'node:http';
import type { TelemetryConfig } from './config';

export interface TelemetryHandle {
  /** False when no endpoint is configured or `RG_OTEL_ENABLED=false` (no-op SDK). */
  readonly enabled: boolean;
  /** Flushes and stops the SDK, bounded by `timeoutMs` (default 5 s). Never throws. */
  shutdown(timeoutMs?: number): Promise<void>;
}

/** Replaces the default exporters/processors; used by tests. */
export interface TelemetryOverrides {
  spanProcessors?: SpanProcessor[];
  logRecordProcessors?: LogRecordProcessor[];
  metricReaders?: IMetricReader[];
}

export const BATCH_QUEUE_SIZE = 2048;
export const SHUTDOWN_TIMEOUT_MS = 5000;
const FAILURE_LOG_INTERVAL_MS = 60_000;

const NOOP_HANDLE: TelemetryHandle = { enabled: false, shutdown: () => Promise.resolve() };

const STATE_KEY = Symbol.for('reviewgraph.api.telemetry');
type GlobalWithState = typeof globalThis & { [STATE_KEY]?: TelemetryHandle };

/** Health, readiness and metrics probes are never traced. */
export function isIgnoredPath(url: string | undefined): boolean {
  const path = (url ?? '').split('?')[0] ?? '';
  return (
    path === '/health' ||
    path.startsWith('/health/') ||
    path.startsWith('/api/v1/health') ||
    path === '/metrics' ||
    path.startsWith('/api/v1/metrics')
  );
}

const QUERY_BEARING_ATTRIBUTES = ['http.target', 'http.url', 'url.full', 'url.query', 'url.path'];

/** Keeps method, route and status only: drops query strings from the server span. */
function stripQuery(span: { setAttribute(k: string, v: string): unknown }, req: unknown): void {
  const url = (req as IncomingMessage).url;
  if (typeof url !== 'string') return;
  const path = url.split('?')[0] ?? '';
  for (const key of QUERY_BEARING_ATTRIBUTES) {
    span.setAttribute(key, key === 'url.query' ? '' : path);
  }
}

let lastFailureLog = 0;

function reportExportFailure(signal: string, error: Error | undefined): void {
  try {
    metrics
      .getMeter('reviewgraph-api')
      .createCounter('otel_export_failures_total')
      .add(1, { signal });
  } catch {
    // Telemetry failures must never change request outcomes.
  }
  const now = Date.now();
  if (now - lastFailureLog >= FAILURE_LOG_INTERVAL_MS) {
    lastFailureLog = now;
    console.error(
      JSON.stringify({
        timestamp: new Date().toISOString(),
        level: 'warn',
        context: 'Telemetry',
        message: `otlp export failed for ${signal}; dropping (logged once per minute)`,
        error: error?.message,
      }),
    );
  }
}

/** Wraps an exporter so failed exports are counted and logged at most once a minute. */
export function trackExportFailures<T extends object>(exporter: T, signal: string): T {
  return new Proxy(exporter, {
    get(target, prop, receiver) {
      const value = Reflect.get(target, prop, receiver) as unknown;
      if (prop !== 'export' || typeof value !== 'function') {
        return typeof value === 'function' ? value.bind(target) : value;
      }
      return (items: unknown, done: (result: ExportResult) => void) =>
        (value as (i: unknown, d: (r: ExportResult) => void) => void).call(
          target,
          items,
          (result) => {
            if (result.code !== ExportResultCode.SUCCESS) reportExportFailure(signal, result.error);
            done(result);
          },
        );
    },
  });
}

function buildSdk(config: TelemetryConfig, overrides: TelemetryOverrides): NodeSDK {
  const base = config.endpoint ?? '';
  const exporterConfig = (path: string) => ({ url: `${base}${path}`, headers: config.headers });
  const periodic = (exporter: OTLPMetricExporter) =>
    new PeriodicExportingMetricReader({
      exporter: trackExportFailures(exporter, 'metrics'),
      exportIntervalMillis: 30_000,
    });

  return new NodeSDK({
    resource: resourceFromAttributes({
      [ATTR_SERVICE_NAME]: config.serviceName,
      [ATTR_SERVICE_VERSION]: config.serviceVersion,
      'deployment.environment.name': config.environment,
    }),
    spanProcessors: overrides.spanProcessors ?? [
      new BatchSpanProcessor(
        trackExportFailures(new OTLPTraceExporter(exporterConfig('/v1/traces')), 'traces'),
        { maxQueueSize: BATCH_QUEUE_SIZE, maxExportBatchSize: 512, scheduledDelayMillis: 5000 },
      ),
    ],
    logRecordProcessors: overrides.logRecordProcessors ?? [
      new BatchLogRecordProcessor({
        exporter: trackExportFailures(new OTLPLogExporter(exporterConfig('/v1/logs')), 'logs'),
        maxQueueSize: BATCH_QUEUE_SIZE,
      }),
    ],
    metricReaders: overrides.metricReaders ?? [
      periodic(new OTLPMetricExporter(exporterConfig('/v1/metrics'))),
    ],
    instrumentations: [
      new HttpInstrumentation({
        ignoreIncomingRequestHook: (req) => isIgnoredPath(req.url),
        requestHook: (span, req) => stripQuery(span, req),
      }),
      // Statements are recorded with parameters stripped.
      new PgInstrumentation({ enhancedDatabaseReporting: false }),
      // Only the command name is recorded, never keys or values.
      new IORedisInstrumentation({ dbStatementSerializer: (command) => command }),
    ],
  });
}

/**
 * Starts the SDK once per process. Returns a no-op handle when telemetry is disabled; repeated
 * calls (dev hot reload, `--import` plus an explicit call) return the first handle.
 */
export function startTelemetry(
  config: TelemetryConfig,
  overrides: TelemetryOverrides = {},
): TelemetryHandle {
  const g = globalThis as GlobalWithState;
  if (g[STATE_KEY]) return g[STATE_KEY];
  if (!config.enabled) {
    g[STATE_KEY] = NOOP_HANDLE;
    return NOOP_HANDLE;
  }

  const sdk = buildSdk(config, overrides);
  sdk.start();
  const handle: TelemetryHandle = {
    enabled: true,
    async shutdown(timeoutMs = SHUTDOWN_TIMEOUT_MS) {
      let timer: NodeJS.Timeout | undefined;
      try {
        await Promise.race([
          sdk.shutdown(),
          new Promise<void>((resolve) => {
            timer = setTimeout(resolve, timeoutMs);
          }),
        ]);
      } catch {
        // Flush errors are swallowed: shutdown must never fail the process exit.
      } finally {
        if (timer) clearTimeout(timer);
        g[STATE_KEY] = undefined;
      }
    },
  };
  g[STATE_KEY] = handle;
  return handle;
}

/** The handle created by `startTelemetry`, or a no-op handle if it never ran. */
export function getTelemetryHandle(): TelemetryHandle {
  return (globalThis as GlobalWithState)[STATE_KEY] ?? NOOP_HANDLE;
}
