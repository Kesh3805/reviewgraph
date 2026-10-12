import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

const SRC = resolve(__dirname, '../../src');
const DOMAIN_DIRS = ['reviews', 'findings', 'publisher'];
const PROVIDER_IMPORT = /from\s+['"][^'"]*(@octokit\/|providers\/github)[^'"]*['"]/;
const REQUIRE_PROVIDER = /require\(\s*['"][^'"]*(@octokit\/|providers\/github)[^'"]*['"]\s*\)/;

function tsFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...tsFiles(path));
    else if (path.endsWith('.ts')) out.push(path);
  }
  return out;
}

export function providerImports(dir: string): string[] {
  return tsFiles(dir).filter((file) => {
    const text = readFileSync(file, 'utf8');
    return PROVIDER_IMPORT.test(text) || REQUIRE_PROVIDER.test(text);
  });
}

describe('domain modules import no provider code (INV-004)', () => {
  it('inv_004_ts_domain_modules_do_not_import_octokit_or_github_adapter', () => {
    const offenders = DOMAIN_DIRS.flatMap((d) => providerImports(join(SRC, d)));
    expect(offenders).toEqual([]);
  });

  it('inv_004_scan_detects_a_provider_import', () => {
    expect(PROVIDER_IMPORT.test("import { Octokit } from '@octokit/rest';")).toBe(true);
    expect(PROVIDER_IMPORT.test("import { X } from '../providers/github/octokit.factory';")).toBe(
      true,
    );
    expect(PROVIDER_IMPORT.test("import { X } from '../providers/ports';")).toBe(false);
  });
});
