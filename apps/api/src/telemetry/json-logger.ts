import type { LoggerService } from '@nestjs/common';
import { trace, type Span } from '@opentelemetry/api';
import { logs, SeverityNumber } from '@opentelemetry/api-logs';
import pino, { type DestinationStream, type Logger } from 'pino';
import { currentRequestId } from '../common/request-id.middleware';
import { CORRELATION_ATTRIBUTES } from './attributes';

/**
 * Redaction injection point. OBS-006 supplies the real formatter; until then only the
 * well-known credential headers/fields are censored.
 */
export const REDACT_PATHS = [
  'authorization',
  'cookie',
  'headers.authorization',
  'headers.cookie',
  'req.headers.authorization',
  'req.headers.cookie',
  'req.headers["x-hub-signature-256"]',
];

/** Fields attached to every line from the active span and request context. */
export function correlationFields(): Record<string, unknown> {
  const fields: Record<string, unknown> = {};
  const span = trace.getActiveSpan();
  if (span) {
    const { traceId, spanId } = span.spanContext();
    fields.trace_id = traceId;
    fields.span_id = spanId;
    const attributes = (span as Span & { attributes?: Record<string, unknown> }).attributes;
    if (attributes) {
      for (const key of CORRELATION_ATTRIBUTES) {
        if (attributes[key] !== undefined) fields[key] = attributes[key];
      }
    }
  }
  const requestId = currentRequestId();
  if (requestId) fields.request_id = requestId;
  return fields;
}

export function createJsonLogger(
  level = process.env.LOG_LEVEL ?? 'info',
  destination?: DestinationStream,
): Logger {
  return pino(
    {
      level,
      base: undefined,
      messageKey: 'message',
      timestamp: () => `,"timestamp":"${new Date().toISOString()}"`,
      formatters: { level: (label) => ({ level: label }) },
      redact: { paths: REDACT_PATHS, censor: '[redacted]' },
      mixin: correlationFields,
    },
    destination,
  );
}

const SEVERITY: Record<string, SeverityNumber> = {
  trace: SeverityNumber.TRACE,
  debug: SeverityNumber.DEBUG,
  info: SeverityNumber.INFO,
  warn: SeverityNumber.WARN,
  error: SeverityNumber.ERROR,
};

/**
 * Nest `LoggerService` writing one JSON line per event and bridging every event to the OTel
 * Logs API (a no-op until the SDK is started with an endpoint).
 */
export class JsonLoggerService implements LoggerService {
  constructor(private readonly logger: Logger = createJsonLogger()) {}

  log(message: unknown, context?: string): void {
    this.emit('info', message, { context });
  }
  error(message: unknown, stackOrContext?: string, context?: string): void {
    this.emit(
      'error',
      message,
      context ? { context, stack: stackOrContext } : { context: stackOrContext },
    );
  }
  warn(message: unknown, context?: string): void {
    this.emit('warn', message, { context });
  }
  debug(message: unknown, context?: string): void {
    this.emit('debug', message, { context });
  }
  verbose(message: unknown, context?: string): void {
    this.emit('trace', message, { context });
  }

  private emit(level: keyof typeof SEVERITY, message: unknown, fields: Record<string, unknown>) {
    const text = typeof message === 'string' ? message : JSON.stringify(message);
    this.logger[level as 'info'](fields, text);
    if (!this.logger.isLevelEnabled(level)) return;
    const attributes = { ...correlationFields() } as Record<string, string>;
    if (fields.context) attributes.context = String(fields.context);
    logs.getLogger('reviewgraph-api').emit({
      severityNumber: SEVERITY[level],
      severityText: level.toUpperCase(),
      body: text,
      attributes,
    });
  }
}
