import { Processor, WorkerHost } from '@nestjs/bullmq';
import { Job } from 'bullmq';

@Processor('email')
export class EmailProcessor extends WorkerHost {
  async process(job: Job<{ to: string }>): Promise<void> {
    console.log(`sending email to ${job.data.to}`);
  }
}
