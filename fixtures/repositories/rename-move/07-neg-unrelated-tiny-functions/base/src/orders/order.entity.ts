export class OrderEntity {
  name = '';
  total = 0;

  summary(): string {
    return `${this.name}: ${this.total.toFixed(2)}`;
  }
}
