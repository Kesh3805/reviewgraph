export class Emitter {
  subscribe(handler: () => void): void {
    this.listeners.push(handler);
  }

  private listeners: Array<() => void> = [];
}