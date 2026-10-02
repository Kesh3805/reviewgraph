import { Injectable } from '@nestjs/common';
import { SpanStatusCode, trace, type Span, type Tracer } from '@opentelemetry/api';
import type { SpanAttributes, StageSpanName } from './attributes';

export const TRACER_NAME = 'reviewgraph-api';

/** Typed wrapper for manual stage spans. */
@Injectable()
export class TracerService {
  private get tracer(): Tracer {
    // Resolved lazily so the global provider registered by the SDK is always used.
    return trace.getTracer(TRACER_NAME);
  }

  /**
   * Runs `fn` inside a child span of the active context. Exceptions are recorded and the
   * span status set to ERROR before the exception is rethrown; the span always ends.
   */
  async withSpan<T>(
    name: StageSpanName,
    attrs: SpanAttributes,
    fn: (span: Span) => Promise<T> | T,
  ): Promise<T> {
    return this.tracer.startActiveSpan(name, async (span) => {
      try {
        span.setAttributes(attrs as Record<string, string | number | boolean>);
        return await fn(span);
      } catch (err) {
        span.recordException(err instanceof Error ? err : new Error(String(err)));
        span.setStatus({
          code: SpanStatusCode.ERROR,
          message: err instanceof Error ? err.message : String(err),
        });
        throw err;
      } finally {
        span.end();
      }
    });
  }
}
