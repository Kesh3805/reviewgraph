#!/usr/bin/env node
// Writes a .trivyignore from the unexpired entries of security/exceptions.toml (CI-008).
//   node scripts/security/exceptions-to-trivyignore.mjs [exceptions.toml] > .trivyignore
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseExceptions } from './check-exceptions.mjs';

/** Ignore lines for entries whose expiry is still in the future; `# reason` comments included. */
export function trivyIgnore(tomlText, now = new Date()) {
  const lines = ['# generated from security/exceptions.toml; do not edit'];
  for (const e of parseExceptions(tomlText)) {
    const expires = Date.parse(e.expires);
    if (!e.id || Number.isNaN(expires) || expires < now.getTime()) continue;
    lines.push(`# ${e.reason ?? ''} (owner ${e.owner ?? '?'}, expires ${e.expires})`, e.id);
  }
  return lines.join('\n') + '\n';
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const file =
    process.argv[2] ?? fileURLToPath(new URL('../../security/exceptions.toml', import.meta.url));
  process.stdout.write(trivyIgnore(existsSync(file) ? readFileSync(file, 'utf8') : ''));
}
