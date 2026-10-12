export interface User {
  id: string;
  role: string;
  active: boolean;
}

export function audit(value: string): void {}

export async function handle(item: string): Promise<void> {}

export async function* stream(): AsyncGenerator<string> {}

export function checkAccess(user: User | null, roles: string[]): boolean {
  if (user === null) {
    return false;
  }
  if (!user.active) {
    throw new Error('inactive');
  } else {
    audit(user.id);
  }
  return roles.includes(user.role);
}

export function label(count: number): string {
  const suffix = count === 1 ? '' : 's';
  switch (count) {
    case 0:
      return 'none';
    default:
      return `${count} item${suffix}`;
  }
}

export async function processAll(items: string[], ids: Record<string, number>): Promise<number> {
  let total = 0;
  for (let i = 0; i < items.length; i++) {
    total += i;
  }
  for (const item of items) {
    await handle(item);
  }
  for (const key in ids) {
    total += ids[key];
  }
  while (total > 100) {
    total -= 10;
  }
  do {
    total++;
  } while (total < 5);
  items.forEach((item) => audit(item));
  for await (const chunk of stream()) {
    total += chunk.length;
  }
  return total;
}

export class Shapes {
  none(): void {
    return;
  }
  nothing(): null {
    return null;
  }
  undef(): undefined {
    return undefined;
  }
  yes(): boolean {
    return true;
  }
  no(): boolean {
    return false;
  }
  lit(): number {
    return 42;
  }
  ident(value: number): number {
    return value;
  }
  obj(): { a: number } {
    return { a: 1 };
  }
  call(): number {
    return Math.max(1, 2);
  }
  expr(a: number): number {
    return a + 1;
  }
}
