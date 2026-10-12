import { Injectable } from '@nestjs/common';
import { NotFoundError } from '../common/errors';
import { User } from './user.entity';

@Injectable()
export class UsersRepository {
  private readonly rows = new Map<string, User>();

  async findUser(id: string): Promise<User> {
    const user = this.rows.get(id);
    if (!user) {
      throw new NotFoundError(`user ${id}`);
    }
    return user;
  }

  async update(id: string, patch: Partial<User>): Promise<User> {
    const user = await this.findUser(id);
    const next = { ...user, ...patch };
    this.rows.set(id, next);
    return next;
  }
}
