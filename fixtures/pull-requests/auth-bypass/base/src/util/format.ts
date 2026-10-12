// Formatting helpers shared by controllers.

export function authorizeHeader(token: string): string {
  return `Bearer ${token}`;
}

export function formatName(first: string, last: string): string {
  return `${first} ${last}`;
}
