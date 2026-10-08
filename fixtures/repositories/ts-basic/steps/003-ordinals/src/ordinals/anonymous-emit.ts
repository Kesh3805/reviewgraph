export function withCallbacks(register: (cb: () => void) => void): void {
  register(() => undefined);
  register(() => undefined);
}