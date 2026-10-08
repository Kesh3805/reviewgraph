import { plainToInstance } from 'class-transformer';

export interface Mailer {
  send(payload: NotificationPayload, options: { priority: string }): void;
}

export interface FileStore {
  save(payload: NotificationPayload): void;
}

export class NotificationPayload {
  constructor(
    readonly userId: string,
    readonly message: string,
  ) {}
}

export class CreateUserDto {
  email = '';
}

export function validateEmail(raw: unknown): void {
  if (typeof raw !== 'object') {
    throw new TypeError('expected an object');
  }
}

export function registerHandlers(): void {}

export class NotificationService {
  constructor(
    private readonly mailer: Mailer,
    private readonly fileStore: FileStore,
  ) {}

  notify(userId: string, message: string): void {
    const payload = new NotificationPayload(userId, message);
    this.mailer.send(payload, { priority: 'high' });
    this.fileStore.save(payload);
    console.log('sent');
  }

  parseInput(raw: unknown): CreateUserDto {
    validateEmail(raw);
    return plainToInstance(CreateUserDto, raw);
  }
}

registerHandlers();
