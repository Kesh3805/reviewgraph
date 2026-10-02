export function add(a: number, b: number): number {
  return a + b;
}

export function mul(a: number, b: number): number {
  if (a === 0 || b === 0) {
    return 0;
  }
  return a * b;
}

export class Calculator {
  private values: number[] = [];

  push(value: number): void {
    this.values.push(value);
  }

  total(): number {
    let sum = 0;
    for (const v of this.values) {
      sum = this.add(sum, v);
    }
    return sum;
  }

  private add(a: number, b: number): number {
    return add(a, b);
  }
}
