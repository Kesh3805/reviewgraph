import {
  CallHandler,
  type ExecutionContext,
  Global,
  Injectable,
  type NestInterceptor,
  type OnApplicationShutdown,
  Module,
} from '@nestjs/common';
import { APP_INTERCEPTOR } from '@nestjs/core';
import { trace } from '@opentelemetry/api';
import type { Request } from 'express';
import type { Observable } from 'rxjs';
import { getTelemetryHandle } from './sdk';
import { TracerService } from './tracer.service';

/** Names the server span `HTTP <method> <route>` once Express has resolved the route. */
@Injectable()
export class HttpRouteInterceptor implements NestInterceptor {
  intercept(context: ExecutionContext, next: CallHandler): Observable<unknown> {
    if (context.getType() === 'http') {
      const req = context.switchToHttp().getRequest<Request & { route?: { path?: string } }>();
      const span = trace.getActiveSpan();
      const route = req.baseUrl + (req.route?.path ?? '');
      if (span && route) {
        span.updateName(`HTTP ${req.method} ${route}`);
        span.setAttribute('http.route', route);
      }
    }
    return next.handle();
  }
}

/** Flushes and stops the SDK after Nest has closed (bounded to 5 s). */
@Injectable()
export class TelemetryLifecycle implements OnApplicationShutdown {
  async onApplicationShutdown(): Promise<void> {
    await getTelemetryHandle().shutdown();
  }
}

@Global()
@Module({
  providers: [
    TracerService,
    TelemetryLifecycle,
    { provide: APP_INTERCEPTOR, useClass: HttpRouteInterceptor },
  ],
  exports: [TracerService],
})
export class TelemetryModule {}
