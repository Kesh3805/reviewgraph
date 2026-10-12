import { Processor, WorkerHost } from '@nestjs/bullmq';

export interface EmailJob {
  id: string;
  data: { to: string; template: string };
}

export interface Mailer {
  send(to: string, template: string): Promise<void>;
}

export interface SentStore {
  has(jobId: string): Promise<boolean>;
  add(jobId: string): Promise<void>;
}

@Processor('email')
export class EmailProcessor extends WorkerHost {
  constructor(
    private readonly mailer: Mailer,
    private readonly sent: SentStore,
  ) {
    super();
  }

  async process(job: EmailJob): Promise<void> {
    if (await this.sent.has(job.id)) {
      return;
    }
    await this.mailer.send(job.data.to, job.data.template);
    await this.sent.add(job.id);
  }
}
