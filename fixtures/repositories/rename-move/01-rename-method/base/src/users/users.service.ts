import { Injectable } from '@nestjs/common';

export interface UserRecord {
  id: string;
  email: string;
  displayName: string;
  active: boolean;
}

@Injectable()
export class UsersService {
  private readonly users: UserRecord[] = [];

  findByEmail(email: string): UserRecord | undefined {
    const normalized = email.trim().toLowerCase();
    return this.users.find((user) => user.active && user.email.toLowerCase() === normalized);
  }

  register(email: string, displayName: string): UserRecord {
    const id = `user-${this.users.length + 1}`;
    const record: UserRecord = { id, email: email.trim(), displayName, active: true };
    this.users.push(record);
    return record;
  }

  deactivate(id: string): boolean {
    const user = this.users.find((candidate) => candidate.id === id);
    if (!user) {
      return false;
    }
    user.active = false;
    return true;
  }
}
