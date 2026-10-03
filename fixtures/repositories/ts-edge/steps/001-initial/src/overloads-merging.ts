export function over(a: string): string;
export function over(a: number): number;
export function over(a: any): any {
  return a;
}

export interface Merged { a: string }
export interface Merged { b: number }

export namespace Merged { export const x = 1; }
