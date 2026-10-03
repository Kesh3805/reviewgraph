import { Global, Module } from '@nestjs/common';
import { APP_GUARD } from '@nestjs/core';
import { MembershipService } from './membership.service';
import { TenancyGuard } from './tenancy.guard';

/**
 * Global tenancy guard. It must be registered after the session guard (AuthModule, API-004),
 * which sets `req.rgUser`: keep AuthModule before TenancyModule in the AppModule imports.
 */
@Global()
@Module({
  providers: [MembershipService, { provide: APP_GUARD, useClass: TenancyGuard }],
  exports: [MembershipService],
})
export class TenancyModule {}
