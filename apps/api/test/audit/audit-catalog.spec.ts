import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { AUDIT_ACTIONS } from '../../src/audit/audit.types';
import { redactValue } from '../../src/common/redact';

const SRC = resolve(__dirname, '../../src');

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? sourceFiles(path) : path.endsWith('.ts') ? [path] : [];
  });
}

describe('audit catalog', () => {
  it('every action written by the API is cataloged', () => {
    const written = new Set<string>();
    for (const file of sourceFiles(SRC)) {
      const text = readFileSync(file, 'utf8');
      if (!text.includes('audit.record') && !text.includes('recordStandalone')) continue;
      for (const m of text.matchAll(
        /action:\s*(?:[^'\n]*\?\s*)?'([a-z_.]+)'(?:\s*:\s*'([a-z_.]+)')?/g,
      )) {
        written.add(m[1]!);
        if (m[2]) written.add(m[2]);
      }
    }
    expect(written.size).toBeGreaterThan(5);
    for (const action of written) {
      expect([action, (AUDIT_ACTIONS as readonly string[]).includes(action)]).toEqual([
        action,
        true,
      ]);
    }
  });

  it('metadata_never_contains_secret_patterns (redaction)', () => {
    const redacted = JSON.stringify(
      redactValue({
        client_secret: 'abc',
        nested: { note: 'token ghp_0123456789abcdefghijklmnopqrstuvwxyzAB here' },
        list: ['Bearer abcdefghijklmnopqrstuvwxyz012345'],
        input_tokens: 42,
        enabled: true,
        verdict: 'useful',
      }),
    );
    expect(redacted).not.toContain('abc"');
    expect(redacted).not.toContain('ghp_');
    expect(redacted).not.toContain('abcdefghijklmnopqrstuvwxyz012345');
    expect(redacted).toContain('"input_tokens":42');
    expect(redacted).toContain('"enabled":true');
    expect(redacted).toContain('"verdict":"useful"');
  });
});
