import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { redactAttributes, redactObject, redactString } from '../../src/telemetry/redact';

const corpus = JSON.parse(
  readFileSync(join(__dirname, '../../../../packages/contracts/redaction-corpus.json'), 'utf8'),
) as {
  positives: { name: string; input: string; expected_redacted: string }[];
  negatives: string[];
};

describe('redaction corpus', () => {
  it.each(corpus.positives)('corpus_positives_are_redacted_ts: $name', (c) => {
    expect(redactString(c.input)).toBe(c.expected_redacted);
  });
  it('corpus_negatives_unchanged_ts', () => {
    for (const n of corpus.negatives) expect(redactString(n)).toBe(n);
  });
  it('redaction_is_idempotent', () => {
    for (const c of corpus.positives) {
      const once = redactString(c.input);
      expect(redactString(once)).toBe(once);
    }
  });
  it('sensitive_keys_redacted_regardless_of_value', () => {
    expect(redactObject({ password: 'x', nested: { Token: 'y' }, ok: 'fine' })).toEqual({
      password: '«redacted:sensitive_key»',
      nested: { Token: '«redacted:sensitive_key»' },
      ok: 'fine',
    });
  });
  it('long_field_is_truncated', () => {
    expect(redactString('a'.repeat(5000)).endsWith('…[truncated]')).toBe(true);
  });
  it('circular objects are safe', () => {
    const a: Record<string, unknown> = {};
    a.self = a;
    expect(() => redactObject(a)).not.toThrow();
  });
  it('span_attributes_scrubbed_before_export', () => {
    expect(redactAttributes({ url: 'ghp_abcdefghijklmnopqrstuvwxyz0123456789', n: 1 })).toEqual({
      url: '«redacted:github_token»',
      n: 1,
    });
  });
});
