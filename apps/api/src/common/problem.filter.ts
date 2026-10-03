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

export interface ProblemDetails {
  type: string;
  title: string;
  status: number;
  detail?: string;
  instance?: string;
  request_id?: string;
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

    const problem: ProblemDetails = {
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

function titleFor(status: number): string {
  return (HttpStatus[status]?.toString() ?? 'error').replace(/_/g, ' ').toLowerCase();
}

function detailOf(exception: HttpException): string | undefined {
  const body = exception.getResponse();
  if (typeof body === 'string') return body;
  const message = (body as { message?: string | string[] }).message;
  return Array.isArray(message) ? message.join('; ') : message;
}
