import { Injectable } from '@nestjs/common';
import { DataSource } from '../common/database';

@Injectable()
export class StockService {
  constructor(private readonly dataSource: DataSource) {}

  async reserve(sku: string, quantity: number): Promise<void> {
    await this.dataSource.manager.query(
      'UPDATE stock SET reserved = reserved + $2 WHERE sku = $1',
      [sku, quantity],
    );
  }

  async release(sku: string, quantity: number): Promise<void> {
    await this.dataSource.manager.query(
      'UPDATE stock SET reserved = reserved - $2 WHERE sku = $1',
      [sku, quantity],
    );
  }
}
