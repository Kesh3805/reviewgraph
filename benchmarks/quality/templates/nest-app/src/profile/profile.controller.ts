import { Controller, Get, Req } from '@nestjs/common';
import { User } from '../users/user.entity';
import { ProfileResponseDto } from './profile.dto';

@Controller('profile')
export class ProfileController {
  @Get()
  me(@Req() req: { user: User & { timezone: string } }): ProfileResponseDto {
    const u = req.user;
    return {
      id: u.id,
      displayName: `${u.firstName} ${u.lastName}`,
      email: u.email,
      timezone: u.timezone,
    };
  }
}
