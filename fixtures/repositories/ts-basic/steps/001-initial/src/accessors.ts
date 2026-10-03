export class Temperature {
  private _celsius = 0;
  get celsius(): number {
    return this._celsius;
  }
  set celsius(value: number) {
    this._celsius = value;
  }
  static get zero(): number {
    return 0;
  }
  [Symbol.iterator]() {}
  "string-key"() {}
}
