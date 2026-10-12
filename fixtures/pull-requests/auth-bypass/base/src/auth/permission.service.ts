import { Injectable } from '@nestjs/common';

@Injectable()
export class PermissionService {
  async check(userId: string, resourceId: string): Promise<boolean> {
    return userId.length > 0 && resourceId.length > 0;
  }
}
