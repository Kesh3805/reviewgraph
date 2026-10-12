import { CanActivate, ExecutionContext, Injectable, SetMetadata } from '@nestjs/common';

export const Roles = (...roles: string[]) => SetMetadata('roles', roles);

@Injectable()
export class RolesGuard implements CanActivate {
  canActivate(context: ExecutionContext): boolean {
    const request = context.switchToHttp().getRequest();
    const roles: string[] = Reflect.getMetadata('roles', context.getHandler()) ?? [];
    return roles.length === 0 || roles.includes(request.user?.role);
  }
}
