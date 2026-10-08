import type { AppConfig } from '../../src/config/config.module';
import type { ConsumeOptions, JobContext, JobHandler, JobQueue } from '../../src/jobs/job-queue';
import type { PublishConsumer } from '../../src/publisher/publish.consumer';
import { PublishWorker } from '../../src/publisher/publish.worker';

describe('PublishWorker', () => {
  const run = '0190f3a2-0000-7000-8000-0000000000aa';

  function setup(config: Partial<AppConfig>) {
    const registered: { queue: string; handler: JobHandler; options?: ConsumeOptions }[] = [];
    const queue = {
      consume: (q: string, handler: JobHandler, options?: ConsumeOptions) => {
        registered.push({ queue: q, handler, options });
        return { stop: () => Promise.resolve() };
      },
    } as unknown as JobQueue;
    const handled: unknown[] = [];
    const consumer = {
      handle: (payload: unknown) => {
        handled.push(payload);
        return Promise.resolve({ outcome: 'published' });
      },
    } as unknown as PublishConsumer;
    const worker = new PublishWorker(queue, consumer, config as AppConfig);
    return { worker, registered, handled };
  }

  it('registers PublishConsumer.handle on review-publish with concurrency 4', async () => {
    const { worker, registered, handled } = setup({ NODE_ENV: 'production' });
    worker.onApplicationBootstrap();
    expect(worker.running).toBe(true);
    expect(registered).toHaveLength(1);
    expect(registered[0]).toMatchObject({ queue: 'review-publish', options: { concurrency: 4 } });
    await registered[0]!.handler({
      jobId: 'j1',
      payload: { review_run_id: run },
    } as unknown as JobContext);
    expect(handled).toEqual([{ review_run_id: run }]);
  });

  it('stays off under NODE_ENV=test unless enabled explicitly', () => {
    const off = setup({ NODE_ENV: 'test' });
    off.worker.onApplicationBootstrap();
    expect(off.registered).toHaveLength(0);
    const on = setup({ NODE_ENV: 'test', QUEUE_CONSUMERS_ENABLED: true });
    on.worker.onApplicationBootstrap();
    expect(on.registered).toHaveLength(1);
  });
});
