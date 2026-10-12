import { Injectable } from '@nestjs/common';

@Injectable()
export class InvoiceService {
  number(sequence: number, year: number): string {
    return `INV-${year}-${String(sequence).padStart(6, '0')}`;
  }

  dueDate(issuedAt: Date, termDays: number): Date {
    const due = new Date(issuedAt.getTime());
    due.setUTCDate(due.getUTCDate() + termDays);
    return due;
  }
}
