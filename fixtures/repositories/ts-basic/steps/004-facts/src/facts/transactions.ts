import { DataSource } from 'typeorm';

export class Account {
  id = '';
  balance = 0;
}

export function notifyLater(accountId: string): void {}

export function Transactional(): MethodDecorator {
  return () => undefined;
}

export class TransferService {
  constructor(private readonly dataSource: DataSource) {}

  async transfer(from: string, to: string, amount: number): Promise<void> {
    await this.dataSource.transaction(async (manager) => {
      await manager.decrement(Account, { id: from }, 'balance', amount);
      await manager.increment(Account, { id: to }, 'balance', amount);
    });
    notifyLater(from);
  }

  @Transactional()
  async archive(id: string): Promise<void> {
    await this.dataSource.query('UPDATE accounts SET archived = true WHERE id = $1', [id]);
  }
}
