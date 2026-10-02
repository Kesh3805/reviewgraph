import { Injectable } from '@nestjs/common';
import { InjectQueue } from '@nestjs/bullmq';
import { Queue } from 'bullmq';
import { UsersRepository } from './users.repository';

@Injectable()
export class UsersService {
  constructor(
    private readonly usersRepository: UsersRepository,
    @InjectQueue('email') private readonly emailQueue: Queue,
  ) {}

  findOne(id: string) {
    return this.usersRepository.findById(id);
  }

  async create(email: string) {
    const user = await this.usersRepository.create(email);
    await this.emailQueue.add('welcome', { to: email });
    return user;
  }
}
