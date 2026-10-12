export function orderLabel(orderId: string, createdAt: Date): string {
  const day = createdAt.toISOString().slice(0, 10);
  return `${orderId.toUpperCase()} (${day})`;
}

export function lineSummary(sku: string, quantity: number): string {
  return quantity === 1 ? `1 x ${sku}` : `${quantity} x ${sku}`;
}
