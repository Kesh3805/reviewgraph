import { Injectable, Logger, BeforeApplicationShutdown } from '@nestjs/common';
import { HttpAdapterHost } from '@nestjs/core';
import type { Server } from 'node:http';

export const DRAIN_TIMEOUT_MS = 25_000;

export type ShutdownHook = () => Promise<void> | void;

/**
 * Coordinates graceful shutdown. Nest calls `beforeApplicationShutdown` before it closes the
 * HTTP server; the server then stops accepting connections and waits for in-flight requests.
 * This service bounds that wait to 25 s and runs registered hooks (the publish consumer from
 * API-007 registers here to stop and release its leases). Pools are closed by their owners in
 * `onApplicationShutdown`, which Nest runs after the server has drained.
 */
@Injectable()
export class ShutdownService implements BeforeApplicationShutdown {
  private readonly logger = new Logger(ShutdownService.name);
  private readonly hooks: { name: string; fn: ShutdownHook }[] = [];
  private drainTimeoutMs = DRAIN_TIMEOUT_MS;

  constructor(private readonly adapterHost: HttpAdapterHost) {}

  /** Registers work to run on shutdown, before the HTTP server is closed. */
  register(name: string, fn: ShutdownHook): void {
    this.hooks.push({ name, fn });
  }

  /** Overrides the drain bound (tests). */
  setDrainTimeout(ms: number): void {
    this.drainTimeoutMs = ms;
  }

  async beforeApplicationShutdown(signal?: string): Promise<void> {
    this.logger.log(`shutting down${signal ? ` on ${signal}` : ''}; draining in-flight requests`);
    const server = this.adapterHost.httpAdapter?.getHttpServer() as Server | undefined;
    if (server) {
      const timer = setTimeout(() => {
        this.logger.warn('drain timeout reached; closing remaining connections');
        server.closeAllConnections();
      }, this.drainTimeoutMs);
      timer.unref();
      // Idle keep-alive sockets would otherwise hold server.close() open.
      server.once('close', () => clearTimeout(timer));
      server.closeIdleConnections();
    }
    for (const { name, fn } of this.hooks) {
      try {
        await fn();
      } catch (err) {
        this.logger.error(`shutdown hook "${name}" failed: ${String(err)}`);
      }
    }
  }
}
