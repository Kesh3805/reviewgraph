export type OrderStatus = 'NEW' | 'PAID' | 'SHIPPED' | 'CANCELLED';

export interface Order {
  id: string;
  organizationId: string;
  status: OrderStatus;
  total: number;
}
