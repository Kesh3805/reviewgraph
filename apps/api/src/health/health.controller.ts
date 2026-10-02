import { Controller, Get, HttpException, HttpStatus, Res } from '@nestjs/common';
import type { Response } from 'express';
import { HealthService, type ReadyReport } from './health.service';

@Controller('health')
export class HealthController {
  constructor(private readonly health: HealthService) {}

  @Get('live')
  live(): { status: 'ok' } {
    if (!this.health.isLive()) {
      throw new HttpException('event loop blocked', HttpStatus.SERVICE_UNAVAILABLE);
    }
    return { status: 'ok' };
  }

  @Get('ready')
  async ready(@Res({ passthrough: true }) res: Response): Promise<ReadyReport> {
    const report = await this.health.ready();
    // The checks stay in the 503 body so operators can see which dependency is down.
    if (report.status !== 'ok') res.status(HttpStatus.SERVICE_UNAVAILABLE);
    return report;
  }
}

/** Unprefixed `GET /health` alias of liveness for simple probes and local use. */
@Controller()
export class HealthAliasController {
  constructor(private readonly health: HealthService) {}

  @Get('health')
  alias(): { status: 'ok' } {
    if (!this.health.isLive()) {
      throw new HttpException('event loop blocked', HttpStatus.SERVICE_UNAVAILABLE);
    }
    return { status: 'ok' };
  }
}
