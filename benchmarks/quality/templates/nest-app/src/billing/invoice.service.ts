import { Injectable } from '@nestjs/common';
import { BillingError } from '../common/errors';
import { ChargeResult, PaymentClient } from './payment.client';

@Injectable()
export class InvoiceService {
  constructor(private readonly payments: PaymentClient) {}

  async settle(customerId: string, amount: number): Promise<ChargeResult | null> {
    try {
      return await this.payments.charge(customerId, amount);
    } catch (e) {
      throw new BillingError('settlement failed', e);
    }
  }

  async retry(customerId: string, amount: number): Promise<boolean> {
    try {
      const result = await this.payments.charge(customerId, amount);
      return result?.ok ?? false;
    } catch {
      return false;
    }
  }
}
