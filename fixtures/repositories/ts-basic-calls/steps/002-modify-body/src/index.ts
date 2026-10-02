import { add as sum, mul, Calculator } from './math';
import { formatNumber, formatLabel as label } from './util';

export function run(): string {
  const calc = new Calculator();
  calc.push(sum(1, 2));
  calc.push(3);
  return label('total', Number(formatNumber(calc.total())));
}

export function scale(value: number, factor: number): number {
  return mul(value, factor);
}
