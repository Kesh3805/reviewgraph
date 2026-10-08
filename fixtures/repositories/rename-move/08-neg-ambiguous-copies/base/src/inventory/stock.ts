export function fillLevel(onHand: number, capacity: number): number {
  return capacity <= 0 ? 0 : (onHand / capacity) * 100;
}
