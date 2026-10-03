/** Postgres `query_canceled`, raised when `statement_timeout` fires. */
const QUERY_CANCELED = '57014';
const CONNECTION_ERRORS = new Set(['ECONNREFUSED', 'ECONNRESET', 'ETIMEDOUT', 'ENOTFOUND']);

export const DB_RETRY_AFTER_SECONDS = 1;

/**
 * True for failures that mean "the database is busy or unreachable, retry later": pool acquire
 * timeout, statement timeout and connection errors. These map to 503 with `Retry-After`.
 */
export function isDbUnavailable(err: unknown): boolean {
  if (!(err instanceof Error)) return false;
  const code = (err as { code?: unknown }).code;
  if (code === QUERY_CANCELED) return true;
  if (typeof code === 'string' && CONNECTION_ERRORS.has(code)) return true;
  // pg-pool's acquire timeout carries no code.
  if (err.message === 'timeout exceeded when trying to connect') return true;
  if (err.message === 'Connection terminated due to connection timeout') return true;
  // Node reports a refused connection to ::1 and 127.0.0.1 as an AggregateError.
  if (err instanceof AggregateError) return err.errors.some(isDbUnavailable);
  return false;
}
