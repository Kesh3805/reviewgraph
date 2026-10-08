export function orderLabel(orderId: string, createdAt: Date): string {
  return `${orderId.toUpperCase()} (${createdAt.toISOString().slice(0, 10)})`;
}

export function lineSummary(sku: string, quantity: number): string {
  return quantity === 1 ? `1 x ${sku}` : `${quantity} x ${sku}`;
}
