import { Injectable } from '@nestjs/common';
import { AuthProvider, Resource, User } from './auth-provider.interface';
import { PermissionService } from './permission.service';

@Injectable()
export class AuthService implements AuthProvider {
  constructor(private readonly permissionService: PermissionService) {}

  async authorize(user: User, resource: Resource): Promise<boolean> {
    return this.permissionService.check(user.id, resource.id);
  }
}
