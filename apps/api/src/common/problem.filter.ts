import {
  ArgumentsHost,
  Catch,
  ExceptionFilter,
  HttpException,
  HttpStatus,
  Logger,
} from '@nestjs/common';
import type { Response } from 'express';
import { DB_RETRY_AFTER_SECONDS, isDbUnavailable } from '../db/errors';
import type { RequestWithId } from './request-id.middleware';

/**
 * An HTTP error that carries RFC 9457 extension members (for example the id of the job that
 * already exists, or `errors` for a validation failure) next to the standard fields.
 */
export class ProblemException extends HttpException {
  constructor(
    status: number,
    readonly detail: string,
    readonly extensions: Record<string, unknown> = {},
    readonly headers: Record<string, string> = {},
  ) {
    super({ message: detail, ...extensions }, status);
  }
}

export interface ProblemDetails {
  type: string;
  title: string;
  status: number;
  detail?: string;
  instance?: string;
  request_id?: string;
  /** RFC 9457 extension members. */
  [extension: string]: unknown;
}

/** Maps every exception to an RFC 9457 `application/problem+json` response. */
@Catch()
export class ProblemFilter implements ExceptionFilter {
  private readonly logger = new Logger(ProblemFilter.name);

  catch(exception: unknown, host: ArgumentsHost): void {
    const http = host.switchToHttp();
    const res = http.getResponse<Response>();
    const req = http.getRequest<RequestWithId>();

    // Pool exhaustion, statement timeouts and connection failures are retryable (API-002).
    const dbUnavailable = !(exception instanceof HttpException) && isDbUnavailable(exception);
    const status =
      exception instanceof HttpException
        ? exception.getStatus()
        : dbUnavailable
          ? HttpStatus.SERVICE_UNAVAILABLE
          : (clientErrorStatus(exception) ?? HttpStatus.INTERNAL_SERVER_ERROR);
    if (dbUnavailable && !res.headersSent) res.setHeader('Retry-After', DB_RETRY_AFTER_SECONDS);
    if (status >= 500) {
      this.logger.error(
        exception instanceof Error ? (exception.stack ?? exception.message) : String(exception),
      );
    }

    const extensions = extensionsOf(exception);
    if (exception instanceof ProblemException && !res.headersSent) {
      for (const [name, value] of Object.entries(exception.headers)) res.setHeader(name, value);
    }
    const problem: ProblemDetails = {
      ...extensions,
      type: 'about:blank',
      title: titleFor(status),
      status,
      // Never leak internals of unexpected errors.
      detail: exception instanceof HttpException ? detailOf(exception) : undefined,
      instance: req.originalUrl,
      request_id: req.requestId,
    };
    if (res.headersSent) return;
    res.status(status).type('application/problem+json').json(problem);
  }
}

/** Body-parser style errors (`http-errors`) carry a 4xx `status`, e.g. 413 for oversized bodies. */
function clientErrorStatus(exception: unknown): number | undefined {
  const status = (exception as { status?: unknown } | null)?.status;
  return typeof status === 'number' && status >= 400 && status < 500 ? status : undefined;
}

/** Extension members: everything an HttpException response carries besides `message`. */
function extensionsOf(exception: unknown): Record<string, unknown> {
  if (!(exception instanceof HttpException)) return {};
  const body = exception.getResponse();
  if (typeof body !== 'object' || body === null) return {};
  if (exception instanceof ProblemException) return { ...exception.extensions };
  // Validation failures (nestjs-zod) list their issues under `errors`.
  const errors = (body as { errors?: unknown }).errors;
  return Array.isArray(errors) ? { errors } : {};
}

function titleFor(status: number): string {
  return (HttpStatus[status]?.toString() ?? 'error').replace(/_/g, ' ').toLowerCase();
}

function detailOf(exception: HttpException): string | undefined {
  const body = exception.getResponse();
  if (typeof body === 'string') return body;
  const message = (body as { message?: string | string[] }).message;
  return Array.isArray(message) ? message.join('; ') : message;
}
