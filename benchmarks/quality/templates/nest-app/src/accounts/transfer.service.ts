import { Injectable } from '@nestjs/common';
import { DataSource } from '../common/database';

@Injectable()
export class TransferService {
  constructor(private readonly dataSource: DataSource) {}

  async transfer(from: string, to: string, amount: number): Promise<void> {
    const runner = this.dataSource.createQueryRunner();
    await runner.startTransaction();
    try {
      await runner.manager.query('UPDATE accounts SET balance = balance - $2 WHERE id = $1', [from, amount]);
      await runner.manager.query('UPDATE accounts SET balance = balance + $2 WHERE id = $1', [to, amount]);
      await runner.commitTransaction();
    } catch (e) {
      await runner.rollbackTransaction();
      throw e;
    } finally {
      await runner.release();
    }
  }
}
