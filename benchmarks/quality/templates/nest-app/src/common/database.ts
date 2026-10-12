export interface EntityManager {
  save<T>(entity: T): Promise<T>;
  query(sql: string, params: unknown[]): Promise<unknown[]>;
}

export interface QueryRunner {
  manager: EntityManager;
  startTransaction(): Promise<void>;
  commitTransaction(): Promise<void>;
  rollbackTransaction(): Promise<void>;
  release(): Promise<void>;
}

export interface DataSource {
  manager: EntityManager;
  transaction<T>(work: (manager: EntityManager) => Promise<T>): Promise<T>;
  createQueryRunner(): QueryRunner;
}
