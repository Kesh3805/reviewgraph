import { Injectable } from '@nestjs/common';
import { EntityManager } from '../common/database';

export interface LedgerEntry {
  accountId: string;
  amount: number;
  memo: string;
}

@Injectable()
export class LedgerRepository {
  async post(manager: EntityManager, entry: LedgerEntry): Promise<LedgerEntry> {
    return manager.save(entry);
  }

  async save(manager: EntityManager, entries: LedgerEntry[]): Promise<void> {
    for (const entry of entries) {
      await manager.save(entry);
    }
  }
}
