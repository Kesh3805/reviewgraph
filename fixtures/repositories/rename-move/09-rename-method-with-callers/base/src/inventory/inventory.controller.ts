import { Body, Controller, Post } from '@nestjs/common';
import { InventoryService } from './inventory.service';

@Controller('inventory')
export class InventoryController {
  constructor(private readonly inventory: InventoryService) {}

  @Post('reserve')
  reserve(@Body() body: { sku: string; quantity: number }): { reserved: boolean } {
    const reserved = this.inventory.reserveStock(body.sku, body.quantity);
    return { reserved };
  }

  @Post('restock')
  restock(@Body() body: { sku: string; quantity: number }): { onHand: number } {
    return { onHand: this.inventory.restock(body.sku, body.quantity) };
  }
}
