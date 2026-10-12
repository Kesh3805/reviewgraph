import { Injectable } from '@nestjs/common';

export interface Parcel {
  weightGrams: number;
  lengthCm: number;
  widthCm: number;
  heightCm: number;
  destination: string;
}

@Injectable()
export class ShippingQuoteCalculator {
  quote(parcel: Parcel, express: boolean): number {
    const billable = Math.max(parcel.weightGrams / 1000, this.volumetricWeight(parcel));
    let price = 5.4;
    if (billable > 2) {
      price += (billable - 2) * 1.2;
    }
    if (!this.isDomestic(parcel)) {
      price *= 1.8;
    }
    if (express) {
      price += 9.9;
    }
    const rounded = Math.round(price * 100) / 100;
    return rounded;
  }

  volumetricWeight(parcel: Parcel): number {
    return (parcel.lengthCm * parcel.widthCm * parcel.heightCm) / 5000;
  }

  isDomestic(parcel: Parcel): boolean {
    return parcel.destination.toUpperCase() === 'DE';
  }
}
