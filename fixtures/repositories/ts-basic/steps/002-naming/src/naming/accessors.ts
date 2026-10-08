export class Account {
  #secret = 1;

  get balance(): number {
    return this.#secret;
  }

  set balance(value: number) {
    this.#secret = value;
  }
}