import { MiddlewareConsumer, Module, NestModule } from '@nestjs/common';
import { ConfigModule } from './config/config.module';
import { ShutdownService } from './common/shutdown.service';
import { requestIdMiddleware } from './common/request-id.middleware';
import { RedisModule } from './common/redis.module';
import { APP_PIPE } from '@nestjs/core';
import { ZodValidationPipe } from 'nestjs-zod';
import { AuditModule } from './audit/audit.service';
import { DbModule } from './db/db.module';
import { JobsModule } from './jobs/job-queue.port';
import { RepositoriesModule } from './repositories/repositories.module';
import { AuthModule } from './auth/auth.module';
import { TenancyModule } from './tenancy/tenancy.module';
import { InternalModule } from './internal/internal.module';
import { HealthModule } from './health/health.module';
import { GithubModule } from './providers/github/github.module';
import { ProvidersModule } from './providers/provider.registry';
import { PublisherModule } from './publisher/publisher.module';
import { WebhooksModule } from './webhooks/webhooks.module';
import { TelemetryModule } from './telemetry/telemetry.module';

@Module({
  imports: [
    ConfigModule,
    TelemetryModule,
    RedisModule,
    DbModule,
    AuditModule,
    JobsModule,
    AuthModule,
    TenancyModule,
    InternalModule,
    ProvidersModule,
    GithubModule,
    WebhooksModule,
    PublisherModule,
    RepositoriesModule,
    HealthModule,
  ],
  providers: [ShutdownService, { provide: APP_PIPE, useClass: ZodValidationPipe }],
})
export class AppModule implements NestModule {
  configure(consumer: MiddlewareConsumer): void {
    consumer.apply(requestIdMiddleware).forRoutes('*path');
  }
}
