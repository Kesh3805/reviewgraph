export function fillLevel(onHand: number, capacity: number): number {
  return capacity <= 0 ? 0 : (onHand / capacity) * 100;
}

export function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, value));
}
