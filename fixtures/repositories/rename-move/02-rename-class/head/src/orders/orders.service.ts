import { Injectable } from '@nestjs/common';

export interface OrderLine {
  sku: string;
  quantity: number;
  unitPrice: number;
}

@Injectable()
export class OrdersService {
  create(customerId: string, lines: OrderLine[]): { customerId: string; lines: OrderLine[] } {
    if (lines.length === 0) {
      throw new Error(`order for ${customerId} has no lines`);
    }
    return { customerId, lines: lines.map((line) => ({ ...line })) };
  }

  total(lines: OrderLine[]): number {
    return lines.reduce((sum, line) => sum + line.quantity * line.unitPrice, 0);
  }

  cancel(reason: string): string {
    const trimmed = reason.trim();
    return trimmed.length > 0 ? `cancelled: ${trimmed}` : 'cancelled';
  }
}
