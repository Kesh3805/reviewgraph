import { Injectable } from '@nestjs/common';
import { InjectRepository } from '@nestjs/typeorm';
import { DataSource, Repository } from 'typeorm';

export class User {
  id = '';
  email = '';
}

@Injectable()
export class UserStore {
  constructor(
    @InjectRepository(User) private readonly users: Repository<User>,
    private readonly dataSource: DataSource,
    private readonly userRepo: any,
  ) {}

  async register(user: User): Promise<User> {
    return this.users.save(user);
  }

  async rename(id: string, email: string): Promise<void> {
    await this.userRepo.update(id, { email });
  }

  async purge(): Promise<void> {
    await this.dataSource.query('DELETE FROM users WHERE active = false');
  }

  async lookup(id: string): Promise<User | null> {
    return this.users.findOne({ where: { id } });
  }

  async deactivateAll(): Promise<void> {
    await this.dataSource.createQueryBuilder().update(User).set({ email: '' }).execute();
  }
}
