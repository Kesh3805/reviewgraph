import { Body, Controller, Param, Patch, Post, Req, UseGuards } from '@nestjs/common';
import { Roles, RolesGuard } from '../auth/roles.guard';
import { User } from '../users/user.entity';
import { AdminService } from './admin.service';

@Controller('admin/users')
export class AdminController {
  constructor(private readonly admin: AdminService) {}

  @Patch(':id')
  update(@Req() req: { user: User }, @Param('id') id: string, @Body() patch: Partial<User>) {
    return this.admin.updateUser(req.user, id, patch);
  }

  @Post(':id/deactivate')
  @UseGuards(RolesGuard)
  @Roles('admin')
  deactivate(@Req() req: { user: User }, @Param('id') id: string) {
    return this.admin.deactivate(req.user, id);
  }
}
