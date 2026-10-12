function trimInput(value: string): string {
  return value.trim();
}

export function clean(value: string): string {
  return trimInput(value).toLowerCase();
}
