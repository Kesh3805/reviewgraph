export function lookup(email: string): string {
  const normalized = email.trim().toLowerCase();
  return normalized;
}
