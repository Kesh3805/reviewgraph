import { Injectable } from '@nestjs/common';
import { DomainError, NotFoundError } from '../common/errors';
import { Order } from './order.entity';
import { OrdersRepository } from './orders.repository';

@Injectable()
export class OrdersService {
  constructor(private readonly orders: OrdersRepository) {}

  async page(organizationId: string, page: number, size: number): Promise<Order[]> {
    return this.orders.listForOrganization(organizationId, page, size);
  }

  async ship(organizationId: string, id: string): Promise<Order> {
    const [order] = await this.orders.find({ where: { id, organizationId }, skip: 0, take: 1 });
    if (!order) {
      throw new NotFoundError(`order ${id}`);
    }
    if (order.status !== 'PAID') {
      throw new DomainError(`order ${id} is ${order.status}`);
    }
    return this.orders.save({ ...order, status: 'SHIPPED' });
  }
}
