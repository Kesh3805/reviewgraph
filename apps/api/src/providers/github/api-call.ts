import { SpanStatusCode, trace } from '@opentelemetry/api';
import { incCounter } from '../../common/metrics';
import { TRACER_NAME } from '../../telemetry/tracer.service';
import { toProviderError } from './errors';

/**
 * Runs one GitHub API call: counts it as `github_api_calls_total{route,status}`, optionally wraps
 * it in a span, and maps failures to `ProviderError`. `route` is the templated route (never a
 * concrete URL), so the metric cardinality stays bounded.
 */
export async function githubCall<T>(
  route: string,
  operation: string,
  fn: () => Promise<{ status: number; data: T }>,
  opts: { span?: string; attributes?: Record<string, string | number> } = {},
): Promise<{ status: number; data: T }> {
  const run = async (): Promise<{ status: number; data: T }> => {
    try {
      const res = await fn();
      incCounter('github_api_calls_total', { route, status: res.status });
      return res;
    } catch (err) {
      const status = (err as { status?: unknown }).status;
      incCounter('github_api_calls_total', {
        route,
        status: typeof status === 'number' ? status : 'network',
      });
      throw toProviderError(err, operation);
    }
  };
  if (!opts.span) return run();
  return trace.getTracer(TRACER_NAME).startActiveSpan(opts.span, async (span) => {
    try {
      if (opts.attributes) span.setAttributes(opts.attributes);
      return await run();
    } catch (err) {
      span.setStatus({
        code: SpanStatusCode.ERROR,
        message: err instanceof Error ? err.message : 'error',
      });
      throw err;
    } finally {
      span.end();
    }
  });
}
