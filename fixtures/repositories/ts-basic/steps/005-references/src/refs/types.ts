export interface Shape extends Base1, Base2 {
  area(): number;
}

export type Pair<T> = { first: T; second: Array<Item> };

export function build(a: Foo, b: Map<string, List<Bar>>): Promise<Baz> {
  return Promise.resolve(null as unknown as Baz);
}

export const user: Api.Models.User | null = null;
