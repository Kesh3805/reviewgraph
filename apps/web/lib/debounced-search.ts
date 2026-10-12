/** Symbol search waits this long after the last keystroke (GX-001). */
export const SEARCH_DEBOUNCE_MS = 200;

/**
 * Debounces calls and aborts the previous request whenever a new one is scheduled, so only the
 * latest query can deliver results.
 */
export class DebouncedSearch<Q, R> {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private controller: AbortController | undefined;

  constructor(
    private readonly fetcher: (query: Q, signal: AbortSignal) => Promise<R>,
    private readonly onResult: (result: R, query: Q) => void,
    private readonly onError: (error: unknown, query: Q) => void,
    private readonly delayMs = SEARCH_DEBOUNCE_MS,
  ) {}

  search(query: Q): void {
    this.cancel();
    this.timer = setTimeout(() => {
      const controller = new AbortController();
      this.controller = controller;
      this.fetcher(query, controller.signal).then(
        (result) => {
          if (!controller.signal.aborted) this.onResult(result, query);
        },
        (error: unknown) => {
          if (!controller.signal.aborted) this.onError(error, query);
        },
      );
    }, this.delayMs);
  }

  /** Drops the pending timer and aborts the in-flight request, if any. */
  cancel(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.controller?.abort();
    this.controller = undefined;
  }
}
