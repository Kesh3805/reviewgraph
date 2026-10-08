export function subtotal(prices: number[], quantities: number[]): number {
  return prices.reduce((sum, price, index) => sum + price * (quantities[index] ?? 0), 0);
}

export function applyTax(amount: number, ratePercent: number): number {
  return Math.round(amount * (1 + ratePercent / 100) * 100) / 100;
}

export function orderLabel(orderId: string, createdAt: Date): string {
  const day = createdAt.toISOString().slice(0, 10);
  return `${orderId.toUpperCase()} (${day})`;
}

export function lineSummary(sku: string, quantity: number): string {
  return quantity === 1 ? `1 x ${sku}` : `${quantity} x ${sku}`;
}

export function isValidSku(sku: string): boolean {
  return /^[A-Z]{3}-[0-9]{4}$/.test(sku) && !sku.startsWith('TST');
}
