import { Injectable } from '@nestjs/common';
import { InventoryService } from '../inventory/inventory.service';

@Injectable()
export class OrdersService {
  constructor(private readonly inventory: InventoryService) {}

  placeOrder(sku: string, quantity: number): string {
    if (!this.inventory.reserveUnits(sku, quantity)) {
      return `rejected: ${sku} is out of stock`;
    }
    return `accepted: ${quantity} x ${sku}`;
  }

  cancelOrder(sku: string, quantity: number): number {
    return this.inventory.restock(sku, quantity);
  }
}
