import { HttpStatus } from '@nestjs/common';
import { ProblemException } from './problem.filter';

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
/** `to_char(ts at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')`: microsecond precision. */
const TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}Z$/;

/**
 * Keyset pagination on `(created_at, id)` descending. The timestamp keeps Postgres microsecond
 * precision (a JS Date would truncate it and make pages skip or repeat rows).
 */
export interface TimeCursor {
  createdAt: string;
  id: string;
}

export function encodeTimeCursor(cursor: TimeCursor): string {
  return Buffer.from(JSON.stringify([cursor.createdAt, cursor.id])).toString('base64url');
}

export function decodeTimeCursor(cursor: string | undefined): TimeCursor | null {
  if (!cursor) return null;
  try {
    const parsed: unknown = JSON.parse(Buffer.from(cursor, 'base64url').toString('utf8'));
    if (
      Array.isArray(parsed) &&
      parsed.length === 2 &&
      typeof parsed[0] === 'string' &&
      TIMESTAMP.test(parsed[0]) &&
      typeof parsed[1] === 'string' &&
      UUID.test(parsed[1])
    ) {
      return { createdAt: parsed[0], id: parsed[1] };
    }
  } catch {
    // fall through
  }
  throw new ProblemException(HttpStatus.BAD_REQUEST, 'invalid cursor');
}

/** Splits a `limit + 1` result into the page and the cursor of the next page. */
export function pageOf<T extends { id: string; cursor_ts: string }>(
  rows: T[],
  limit: number,
): { page: T[]; next_cursor: string | null } {
  const page = rows.slice(0, limit);
  const last = page.at(-1);
  return {
    page,
    next_cursor:
      rows.length > limit && last
        ? encodeTimeCursor({ createdAt: last.cursor_ts, id: last.id })
        : null,
  };
}
