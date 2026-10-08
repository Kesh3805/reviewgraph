import { Body, Controller, Param, Put } from '@nestjs/common';
import { AdminService } from '../admin/admin.service';
import { UpdateUserDto } from './update-user.dto';
import { User } from './user.entity';

@Controller('users')
export class UserController {
  constructor(private readonly adminService: AdminService) {}

  @Put(':id')
  async update(@Param('id') id: string, @Body() dto: UpdateUserDto): Promise<User> {
    const actor = { id, role: 'member' } as User;
    const target = { id } as User;
    return this.adminService.updateUser(actor, target, dto);
  }
}
