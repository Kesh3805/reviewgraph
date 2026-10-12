import { Injectable } from '@nestjs/common';

@Injectable()
export class InventoryService {
  private readonly stock = new Map<string, number>();

  reserveUnits(sku: string, quantity: number): boolean {
    const available = this.stock.get(sku) ?? 0;
    if (available < quantity) {
      return false;
    }
    this.stock.set(sku, available - quantity);
    return true;
  }

  restock(sku: string, quantity: number): number {
    const next = (this.stock.get(sku) ?? 0) + quantity;
    this.stock.set(sku, next);
    return next;
  }
}
