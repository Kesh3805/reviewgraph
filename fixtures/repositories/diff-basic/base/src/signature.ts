export function search(email: string): string | undefined {
  const normalized = email.trim().toLowerCase();
  return normalized;
}
