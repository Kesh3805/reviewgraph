export function subtotal(prices: number[], quantities: number[]): number {
  return prices.reduce((sum, price, index) => sum + price * (quantities[index] ?? 0), 0);
}

export function applyTax(amount: number, ratePercent: number): number {
  return Math.round(amount * (1 + ratePercent / 100) * 100) / 100;
}
