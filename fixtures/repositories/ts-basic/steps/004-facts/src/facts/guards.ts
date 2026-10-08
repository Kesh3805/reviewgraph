import { Controller, Get, UseGuards } from '@nestjs/common';
import { AuthGuard } from './auth.guard';
import { Public, RequirePermission, Roles } from './decorators';

@Controller('admin')
@UseGuards(AuthGuard)
export class AdminController {
  @Get('users')
  @Roles('admin')
  list(): string[] {
    return [];
  }

  @Public()
  @Get('health')
  health(): string {
    return 'ok';
  }

  @RequirePermission('audit:read')
  @Get('audit')
  audit(): string {
    return 'audit';
  }
}
