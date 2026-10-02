import { OrderService, computeTotal } from './a/orders';

const service = new OrderService();
const lines = [{ sku: 'A-1', quantity: 2, price: 10 }];

export const preview = computeTotal(lines);
export const submitted = service.submit(lines);
