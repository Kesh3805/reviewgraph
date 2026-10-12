import { Injectable } from '@nestjs/common';
import { ForbiddenError } from '../common/errors';
import { User } from '../users/user.entity';
import { PermissionService, Resource } from './permission.service';

@Injectable()
export class AuthService {
  constructor(private readonly permissions: PermissionService) {}

  async authorize(user: User | undefined, resource: Resource): Promise<void> {
    if (!user) {
      throw new ForbiddenError('anonymous');
    }
    await this.permissions.check(user, resource);
  }
}
