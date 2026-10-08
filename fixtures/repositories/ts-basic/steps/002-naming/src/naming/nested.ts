export namespace Billing {
  export class Payment {
    capture(amount: number): void {}
  }
}

export function inBody(): void {
  class NotASymbol {}
  void NotASymbol;
}