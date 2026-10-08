import { ForbiddenException, Injectable } from '@nestjs/common';
import { InjectRepository } from '@nestjs/typeorm';
import { Repository } from 'typeorm';
import { AuthService } from '../auth/auth.service';
import { UpdateUserDto } from '../users/update-user.dto';
import { User } from '../users/user.entity';

@Injectable()
export class AdminService {
  constructor(
    private readonly auth: AuthService,
    @InjectRepository(User) private readonly userRepo: Repository<User>,
  ) {}

  async updateUser(actor: User, target: User, dto: UpdateUserDto): Promise<User> {
    if (!(await this.auth.authorize(actor, { id: target.id }))) {
      throw new ForbiddenException();
    }
    Object.assign(target, dto);
    return this.userRepo.save(target);
  }
}
