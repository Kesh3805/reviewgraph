import { Injectable } from '@nestjs/common';
import { PermissionService } from '../auth/permission.service';

@Injectable()
export class ReportService {
  constructor(private readonly permissions: PermissionService) {}

  async canExport(userId: string, reportId: string): Promise<boolean> {
    return this.permissions.check(userId, reportId);
  }
}
