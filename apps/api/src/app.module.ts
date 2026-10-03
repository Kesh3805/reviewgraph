import { MiddlewareConsumer, Module, NestModule } from '@nestjs/common';
import { ConfigModule } from './config/config.module';
import { ShutdownService } from './common/shutdown.service';
import { requestIdMiddleware } from './common/request-id.middleware';
import { RedisModule } from './common/redis.module';
import { DbModule } from './db/db.module';
import { AuthModule } from './auth/auth.module';
import { TenancyModule } from './tenancy/tenancy.module';
import { InternalModule } from './internal/internal.module';
import { HealthModule } from './health/health.module';
import { GithubModule } from './providers/github/github.module';
import { ProvidersModule } from './providers/provider.registry';
import { WebhooksModule } from './webhooks/webhooks.module';
import { TelemetryModule } from './telemetry/telemetry.module';

@Module({
  imports: [
    ConfigModule,
    TelemetryModule,
    RedisModule,
    DbModule,
    AuthModule,
    TenancyModule,
    InternalModule,
    ProvidersModule,
    GithubModule,
    WebhooksModule,
    HealthModule,
  ],
  providers: [ShutdownService],
})
export class AppModule implements NestModule {
  configure(consumer: MiddlewareConsumer): void {
    consumer.apply(requestIdMiddleware).forRoutes('*path');
  }
}
