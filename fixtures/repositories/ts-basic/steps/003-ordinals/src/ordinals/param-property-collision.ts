export class Repository {
  readonly name = "orders";

  constructor(private readonly name: string, public readonly id: number) {}
}