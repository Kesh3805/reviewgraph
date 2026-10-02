import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

/** Loads a golden GitHub payload from `test/fixtures/github`. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function fixture(name: string): any {
  return JSON.parse(readFileSync(resolve(__dirname, '../fixtures/github', name), 'utf8'));
}
