import { add as sum, Calculator } from './math';
import { formatNumber, formatLabel as label } from './util';

export function run(): string {
  const calc = new Calculator();
  calc.push(sum(1, 2));
  calc.push(3);
  return label('total', Number(formatNumber(calc.total())));
}
