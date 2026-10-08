import { Module } from '@nestjs/common';
import { FindingsController } from './findings.controller';
import { FindingsService } from './findings.service';
import { FindingTraceService } from './trace.service';

@Module({
  controllers: [FindingsController],
  providers: [FindingsService, FindingTraceService],
})
export class FindingsModule {}
