import { Injectable } from '@nestjs/common';
import { BillingError } from '../common/errors';

export interface ChargeResult {
  id: string;
  ok: boolean;
}

@Injectable()
export class PaymentClient {
  async charge(customerId: string, amount: number): Promise<ChargeResult | null> {
    if (amount <= 0) {
      return null;
    }
    const id = `ch_${customerId}_${amount}`;
    if (customerId.length === 0) {
      throw new BillingError('missing customer');
    }
    return { id, ok: true };
  }
}
