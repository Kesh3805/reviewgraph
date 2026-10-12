import { Injectable } from '@nestjs/common';

@Injectable({ scope: 'singleton' })
export class Mailer {
  send(to: string): string {
    return to.trim();
  }
}
