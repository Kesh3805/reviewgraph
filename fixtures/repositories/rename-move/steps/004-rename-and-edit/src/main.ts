import { OrderService, calculateTotal } from './b/orders';

const service = new OrderService();
const lines = [{ sku: 'A-1', quantity: 2, price: 10 }];

export const preview = calculateTotal(lines);
export const submitted = service.place(lines);
