import { capitalize, formatMoney } from '../util/strings';

export function greet(name: string, balance: number): string {
  return `Hello ${capitalize(name)}, your balance is ${formatMoney(balance, 'EUR')}`;
}
