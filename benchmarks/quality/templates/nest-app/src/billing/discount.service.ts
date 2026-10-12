import { Injectable } from '@nestjs/common';

export interface Cart {
  subtotal: number;
  loyaltyYears: number;
}

@Injectable()
export class DiscountService {
  rate(cart: Cart): number {
    if (cart.loyaltyYears >= 5) {
      return 0.1;
    }
    if (cart.subtotal >= 500) {
      return 0.05;
    }
    return 0;
  }

  total(cart: Cart): number {
    return Math.round(cart.subtotal * (1 - this.rate(cart)) * 100) / 100;
  }
}
