import * as path from 'path';
import Config from './config';

export class Chains {
  private readonly cache = new Map<string, number>();

  constructor(private readonly repo: Repository<User>) {}

  run(qb: QueryBuilder, input?: Input) {
    this.repo.save(input);
    this.cache.get('k');
    qb.where('a').andWhere('b');
    path.join('a', 'b');
    Config.load();
    input?.value.trim();
    Registry.create();
  }
}
