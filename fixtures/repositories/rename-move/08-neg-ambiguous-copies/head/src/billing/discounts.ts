export function discountedPrice(price: number, percent: number): number {
  return price - (price * percent) / 100;
}

export function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, value));
}
