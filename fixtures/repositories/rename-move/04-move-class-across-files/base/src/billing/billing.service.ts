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

@Injectable()
export class PaymentService {
  charge(accountId: string, amountCents: number): { accountId: string; amountCents: number } {
    if (amountCents <= 0) {
      throw new Error(`refusing to charge ${amountCents} to ${accountId}`);
    }
    return { accountId, amountCents };
  }

  refund(paymentId: string, amountCents: number): string {
    const rounded = Math.round(amountCents);
    return `refund ${paymentId} for ${rounded} cents`;
  }
}
