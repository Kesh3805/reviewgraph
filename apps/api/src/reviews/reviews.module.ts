import { Module } from '@nestjs/common';
import { ReviewOrchestrationModule } from './orchestration.module';
import { ReviewsController } from './reviews.controller';
import { ReviewsService } from './reviews.service';

/** Reviews API (API-009); manual triggers go through SUP-001's `SupersessionService`. */
@Module({
  imports: [ReviewOrchestrationModule],
  controllers: [ReviewsController],
  providers: [ReviewsService],
})
export class ReviewsModule {}
