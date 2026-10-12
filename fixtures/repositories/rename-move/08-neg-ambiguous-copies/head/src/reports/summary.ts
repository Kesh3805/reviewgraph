export function completionRate(done: number, total: number): number {
  return total === 0 ? 0 : (done / total) * 100;
}
