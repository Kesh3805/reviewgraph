import { Injectable } from '@nestjs/common';

@Injectable()
export class Mailer {
  send(to: string): string {
    return to.trim();
  }
}
