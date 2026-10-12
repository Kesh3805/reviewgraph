import { Controller, Get, Param, Post, Query, Req } from '@nestjs/common';
import { User } from '../users/user.entity';
import { OrdersService } from './orders.service';

@Controller('orders')
export class OrdersController {
  constructor(private readonly orders: OrdersService) {}

  @Get()
  list(@Req() req: { user: User }, @Query('page') page = 0, @Query('size') size = 20) {
    return this.orders.page(req.user.organizationId, Number(page), Number(size));
  }

  @Post(':id/ship')
  ship(@Req() req: { user: User }, @Param('id') id: string) {
    return this.orders.ship(req.user.organizationId, id);
  }
}
