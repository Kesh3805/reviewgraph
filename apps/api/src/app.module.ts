import { MiddlewareConsumer, Module, NestModule } from '@nestjs/common';
import { ConfigModule } from './config/config.module';
import { ShutdownService } from './common/shutdown.service';
import { requestIdMiddleware } from './common/request-id.middleware';
import { HealthModule } from './health/health.module';
import { TelemetryModule } from './telemetry/telemetry.module';

@Module({
  imports: [ConfigModule, TelemetryModule, HealthModule],
  providers: [ShutdownService],
})
export class AppModule implements NestModule {
  configure(consumer: MiddlewareConsumer): void {
    consumer.apply(requestIdMiddleware).forRoutes('*path');
  }
}
