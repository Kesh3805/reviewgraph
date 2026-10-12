import { Module } from '@nestjs/common';
import { AdminController } from './admin/admin.controller';
import { AdminService } from './admin/admin.service';
import { AuthService } from './auth/auth.service';
import { PermissionService } from './auth/permission.service';
import { OrdersController } from './orders/orders.controller';
import { OrdersRepository } from './orders/orders.repository';
import { OrdersService } from './orders/orders.service';
import { ProfileController } from './profile/profile.controller';
import { UsersRepository } from './users/users.repository';
import { UsersService } from './users/users.service';

@Module({
  controllers: [AdminController, OrdersController, ProfileController],
  providers: [
    AdminService,
    AuthService,
    PermissionService,
    OrdersRepository,
    OrdersService,
    UsersRepository,
    UsersService,
  ],
})
export class AppModule {}
