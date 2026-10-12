export function completionRate(done: number, total: number): number {
  return total === 0 ? 0 : (done / total) * 100;
}

export function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, value));
}
