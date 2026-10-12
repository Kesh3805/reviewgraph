export const SUPPORTED_LOCALES = ['en-US', 'de-DE', 'fr-FR', 'es-ES'];

export function capitalize(value: string): string {
  if (value.length === 0) {
    return value;
  }
  return value.charAt(0).toUpperCase() + value.slice(1);
}

export function slugify(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

export function formatMoney(amount: number, currency: string): string {
  const formatter = new Intl.NumberFormat(SUPPORTED_LOCALES[0], { style: 'currency', currency });
  return formatter.format(amount);
}
