import { Module } from '@nestjs/common';
import { UsersModule } from './users/users.module';
import { EmailProcessor } from './jobs/email.processor';

@Module({
  imports: [UsersModule],
  providers: [EmailProcessor],
})
export class AppModule {}
