import { AuthGuard } from '@nestjs/passport';

export class JwtGuard extends AuthGuard('jwt') implements CanActivate, Disposable {}

export class Plain extends Base {}
