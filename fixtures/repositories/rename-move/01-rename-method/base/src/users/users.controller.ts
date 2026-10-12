import { Controller, Get, Query } from '@nestjs/common';
import { UserRecord, UsersService } from './users.service';

@Controller('users')
export class UsersController {
  constructor(private readonly usersService: UsersService) {}

  @Get('lookup')
  lookup(@Query('email') email: string): UserRecord | null {
    const user = this.usersService.findByEmail(email);
    return user ?? null;
  }

  @Get('health')
  health(): { status: string } {
    return { status: 'ok' };
  }
}
