export function normalize(email: string): string {
  const normalized = email.trim().toLowerCase();
  return normalized;
}
