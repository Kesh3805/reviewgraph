import { z } from 'zod';
import {
  FindingDetailSchema,
  FindingListSchema,
  FindingTraceSchema,
} from '../../src/findings/dto/finding.dto';
import { evidenceItems } from '../../src/findings/finding-rows';

/** Every property name anywhere in a JSON Schema. */
function propertyNames(schema: unknown, out = new Set<string>()): Set<string> {
  if (Array.isArray(schema)) {
    for (const s of schema) propertyNames(s, out);
  } else if (schema && typeof schema === 'object') {
    const obj = schema as Record<string, unknown>;
    if (obj.properties && typeof obj.properties === 'object') {
      for (const key of Object.keys(obj.properties)) out.add(key);
    }
    for (const value of Object.values(obj)) propertyNames(value, out);
  }
  return out;
}

describe('finding response schemas', () => {
  it('trace_excludes_prompt_and_raw_output (schema)', () => {
    for (const schema of [FindingDetailSchema, FindingTraceSchema, FindingListSchema]) {
      const names = propertyNames(z.toJSONSchema(schema, { unrepresentable: 'any' }));
      expect(names.size).toBeGreaterThan(5);
      for (const forbidden of ['prompt', 'raw_output', 'reasoning_artifacts', 'excerpt']) {
        expect(names.has(forbidden)).toBe(false);
      }
    }
  });

  it('evidence items keep only the typed fields', () => {
    const [item] = evidenceItems([
      {
        kind: 'changed_source',
        claim: 'x',
        prompt: 'nope',
        raw_output: 'nope',
        excerpt: 'const secret = 1',
      },
      { claim: 'no kind: dropped' },
    ]);
    expect(Object.keys(item!).sort()).toEqual(
      [
        'claim',
        'claimed_strength',
        'kind',
        'location',
        'origin',
        'relation',
        'symbols',
        'verification',
      ].sort(),
    );
    expect(evidenceItems('not an array')).toEqual([]);
  });
});
