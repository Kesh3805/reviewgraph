import { Injectable } from '@nestjs/common';
import { DataSource } from '../common/database';
import { LedgerEntry, LedgerRepository } from './ledger.repository';

@Injectable()
export class LedgerService {
  constructor(
    private readonly dataSource: DataSource,
    private readonly ledger: LedgerRepository,
  ) {}

  async close(accountId: string, balance: number): Promise<void> {
    await this.dataSource.transaction(async (manager) => {
      await this.ledger.post(manager, { accountId, amount: -balance, memo: 'close' });
      await manager.query('UPDATE accounts SET closed = true WHERE id = $1', [accountId]);
    });
  }

  async import(entries: LedgerEntry[]): Promise<void> {
    await this.dataSource.transaction(async (manager) => {
      await this.ledger.save(manager, entries);
    });
  }

  async reverse(entries: LedgerEntry[]): Promise<void> {
    const reversed = entries.map((e) => ({ ...e, amount: -e.amount, memo: `reverse ${e.memo}` }));
    await this.dataSource.transaction(async (manager) => {
      await this.ledger.save(manager, reversed);
    });
  }
}
