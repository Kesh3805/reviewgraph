import { Module } from '@nestjs/common';
import { FeedbackController } from './feedback.controller';
import { FeedbackService } from './feedback.service';
import { FindingsController } from './findings.controller';
import { FindingsService } from './findings.service';
import { SUPPRESSION_WRITER, UnwiredSuppressionWriter } from './suppression.port';
import { FindingTraceService } from './trace.service';

@Module({
  controllers: [FindingsController, FeedbackController],
  providers: [
    FindingsService,
    FindingTraceService,
    FeedbackService,
    // POL-006 replaces this with the `suppressions` table writer.
    { provide: SUPPRESSION_WRITER, useClass: UnwiredSuppressionWriter },
  ],
  exports: [FeedbackService],
})
export class FindingsModule {}
