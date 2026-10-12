import { capitalize, formatMoney } from '../common/strings';

export function greet(name: string, balance: number): string {
  return `Hello ${capitalize(name)}, your balance is ${formatMoney(balance, 'EUR')}`;
}
