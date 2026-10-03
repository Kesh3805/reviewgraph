import { Module } from '@nestjs/common';
import { GithubModule } from '../providers/github/github.module';
import { HealthAliasController, HealthController } from './health.controller';
import { DependencyProbes, HEALTH_PROBES } from './health.probes';
import { HealthService } from './health.service';

@Module({
  imports: [GithubModule],
  controllers: [HealthController, HealthAliasController],
  providers: [
    HealthService,
    DependencyProbes,
    { provide: HEALTH_PROBES, useExisting: DependencyProbes },
  ],
  exports: [HealthService],
})
export class HealthModule {}
