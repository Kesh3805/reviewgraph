export class OrderEntity {
  name = '';
  total = 0;

  getName() {
    return this.name;
  }

  summary(): string {
    return `${this.name}: ${this.total.toFixed(2)}`;
  }
}
