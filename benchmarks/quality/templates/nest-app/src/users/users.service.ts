import { Injectable } from '@nestjs/common';
import { UsersRepository } from './users.repository';

@Injectable()
export class UsersService {
  constructor(private readonly users: UsersRepository) {}

  async displayName(id: string): Promise<string> {
    const user = await this.users.findUser(id);
    return `${user.firstName} ${user.lastName}`;
  }

  async contactEmail(id: string): Promise<string> {
    const user = await this.users.findUser(id);
    return user.email.toLowerCase();
  }
}
