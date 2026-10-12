export function isValidSku(sku: string): boolean {
  return /^[A-Z]{3}-[0-9]{4}$/.test(sku) && !sku.startsWith('TST');
}
