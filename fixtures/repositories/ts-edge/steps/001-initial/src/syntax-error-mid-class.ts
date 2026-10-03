export class Before {
  ok(): number {
    return 1;
  }
}

export class Broken {
  first(): string {
    return "a";
  }
  second( {
    return 2;
  }
  third(): string {
    return "c";
  }
}

export function after(): number {
  return 3;
}
