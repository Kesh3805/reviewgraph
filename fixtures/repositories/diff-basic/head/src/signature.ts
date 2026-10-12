export function search(email: string, includeDeleted = false): string | undefined {
  const normalized = email.trim().toLowerCase();
  return normalized;
}
