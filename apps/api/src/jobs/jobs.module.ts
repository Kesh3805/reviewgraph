import { Global, Module } from '@nestjs/common';
import { JOB_CANCELLER } from '../common/job-canceller';
import { JOB_QUEUE } from './job-queue';
import { PgJobQueue } from './pg-job-queue';

/** The shared PostgreSQL job queue (API-007), available to every module. */
@Global()
@Module({
  providers: [
    PgJobQueue,
    { provide: JOB_QUEUE, useExisting: PgJobQueue },
    { provide: JOB_CANCELLER, useExisting: PgJobQueue },
  ],
  exports: [PgJobQueue, JOB_QUEUE, JOB_CANCELLER],
})
export class JobsModule {}
