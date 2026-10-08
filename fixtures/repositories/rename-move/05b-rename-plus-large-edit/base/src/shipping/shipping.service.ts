import { Injectable } from '@nestjs/common';

export interface Parcel {
  weightGrams: number;
  lengthCm: number;
  widthCm: number;
  heightCm: number;
  destination: string;
}

@Injectable()
export class ShippingCalculator {
  quote(parcel: Parcel, express: boolean): number {
    const billable = Math.max(parcel.weightGrams / 1000, this.volumetricWeight(parcel));
    let price = 4.9;
    if (billable > 2) {
      price += (billable - 2) * 1.2;
    }
    if (!this.isDomestic(parcel)) {
      price *= 1.8;
    }
    if (express) {
      price += 7.5;
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
