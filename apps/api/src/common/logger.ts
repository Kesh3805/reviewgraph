import { LoggerService } from '@nestjs/common';
import pino, { type Logger } from 'pino';
import { currentRequestId } from './request-id.middleware';

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

export function createPino(level = process.env.LOG_LEVEL ?? 'info'): Logger {
  return pino({
    level,
    base: undefined,
    timestamp: pino.stdTimeFunctions.isoTime,
    formatters: { level: (label) => ({ level: label }) },
    redact: { paths: REDACT_PATHS, censor: '[redacted]' },
    mixin: () => {
      const request_id = currentRequestId();
      return request_id ? { request_id } : {};
    },
  });
}

/** Nest `LoggerService` backed by pino; one JSON line per event. */
export class PinoLoggerService implements LoggerService {
  constructor(private readonly logger: Logger = createPino()) {}

  log(message: unknown, context?: string): void {
    this.logger.info({ context }, format(message));
  }
  error(message: unknown, stackOrContext?: string, context?: string): void {
    const fields = context ? { context, stack: stackOrContext } : { context: stackOrContext };
    this.logger.error(fields, format(message));
  }
  warn(message: unknown, context?: string): void {
    this.logger.warn({ context }, format(message));
  }
  debug(message: unknown, context?: string): void {
    this.logger.debug({ context }, format(message));
  }
  verbose(message: unknown, context?: string): void {
    this.logger.trace({ context }, format(message));
  }
}

function format(message: unknown): string {
  return typeof message === 'string' ? message : JSON.stringify(message);
}
