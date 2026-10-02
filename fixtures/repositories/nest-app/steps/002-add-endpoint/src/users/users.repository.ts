import { Injectable } from '@nestjs/common';
import { InjectRepository } from '@nestjs/typeorm';
import { Repository } from 'typeorm';
import { User } from './user.entity';

@Injectable()
export class UsersRepository {
  constructor(@InjectRepository(User) private readonly users: Repository<User>) {}

  findById(id: string) {
    return this.users.findOne({ where: { id } });
  }

  create(email: string) {
    return this.users.save(this.users.create({ email }));
  }
}
