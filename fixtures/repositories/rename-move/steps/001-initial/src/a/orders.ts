export interface OrderLine {
  sku: string;
  quantity: number;
  price: number;
}

export function computeTotal(lines: OrderLine[]): number {
  let total = 0;
  for (const line of lines) {
    total += line.quantity * line.price;
  }
  return total;
}

export class OrderService {
  private readonly submitted: OrderLine[][] = [];

  submit(lines: OrderLine[]): number {
    const total = computeTotal(lines);
    this.submitted.push(lines);
    return total;
  }

  count(): number {
    return this.submitted.length;
  }
}
