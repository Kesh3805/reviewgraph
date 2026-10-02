import { AsyncLocalStorage } from 'node:async_hooks';
import type { NextFunction, Request, Response } from 'express';
import { v7 as uuidv7 } from 'uuid';

export const REQUEST_ID_HEADER = 'x-request-id';
const MAX_LENGTH = 128;
const SAFE = /^[A-Za-z0-9._:-]+$/;

const store = new AsyncLocalStorage<{ requestId: string }>();

/** The request id of the request currently being handled, if any. */
export function currentRequestId(): string | undefined {
  return store.getStore()?.requestId;
}

/** Returns the incoming `x-request-id` when well-formed, otherwise a fresh UUIDv7. */
export function resolveRequestId(incoming: string | undefined): string {
  if (incoming && incoming.length <= MAX_LENGTH && SAFE.test(incoming)) return incoming;
  return uuidv7();
}

/** Express request carrying the resolved request id. */
export type RequestWithId = Request & { requestId?: string };

export function requestIdMiddleware(req: RequestWithId, res: Response, next: NextFunction): void {
  const header = req.headers[REQUEST_ID_HEADER];
  const requestId = resolveRequestId(Array.isArray(header) ? header[0] : header);
  req.requestId = requestId;
  res.setHeader(REQUEST_ID_HEADER, requestId);
  store.run({ requestId }, next);
}
