import { Injectable } from '@nestjs/common';
import { ForbiddenError } from '../common/errors';
import { User } from '../users/user.entity';

export interface Resource {
  ownerId: string;
  organizationId: string;
}

@Injectable()
export class PermissionService {
  async check(user: User, resource: Resource): Promise<void> {
    if (user.role === 'admin' && user.organizationId === resource.organizationId) {
      return;
    }
    if (user.id !== resource.ownerId) {
      throw new ForbiddenError('not allowed');
    }
  }
}
