import { Injectable } from '@nestjs/common';
import { Order } from './order.entity';

export interface FindOptions {
  where: Partial<Order>;
  skip: number;
  take: number;
}

@Injectable()
export class OrdersRepository {
  private readonly rows: Order[] = [];

  async find(options: FindOptions): Promise<Order[]> {
    const matches = this.rows.filter((o) =>
      Object.entries(options.where).every(([k, v]) => o[k as keyof Order] === v),
    );
    return matches.slice(options.skip, options.skip + options.take);
  }

  async listForOrganization(organizationId: string, page: number, size: number): Promise<Order[]> {
    return this.find({ where: { organizationId }, skip: page * size, take: size });
  }

  async save(order: Order): Promise<Order> {
    const i = this.rows.findIndex((o) => o.id === order.id);
    if (i >= 0) {
      this.rows[i] = order;
    } else {
      this.rows.push(order);
    }
    return order;
  }
}
