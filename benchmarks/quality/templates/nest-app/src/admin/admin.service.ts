import { Injectable } from '@nestjs/common';
import { AuthService } from '../auth/auth.service';
import { User } from '../users/user.entity';
import { UsersRepository } from '../users/users.repository';

@Injectable()
export class AdminService {
  constructor(
    private readonly auth: AuthService,
    private readonly users: UsersRepository,
  ) {}

  async updateUser(actor: User, targetId: string, patch: Partial<User>): Promise<User> {
    const target = await this.users.findUser(targetId);
    await this.auth.authorize(actor, { ownerId: target.id, organizationId: target.organizationId });
    return this.users.update(targetId, patch);
  }

  async deactivate(actor: User, targetId: string): Promise<User> {
    if (actor.role !== 'admin') {
      throw new Error('admins only');
    }
    return this.users.update(targetId, { role: 'member' });
  }
}
