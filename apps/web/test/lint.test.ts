import { ESLint } from 'eslint';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const cwd = fileURLToPath(new URL('..', import.meta.url));

async function lint(code: string) {
  const eslint = new ESLint({ cwd });
  const [result] = await eslint.lintText(code, { filePath: `${cwd}/components/example.tsx` });
  return result?.messages ?? [];
}

describe('lint rules', () => {
  it('no_dangerously_set_inner_html_lint', async () => {
    const bad = await lint(
      'export const X = ({ html }: { html: string }) => <div dangerouslySetInnerHTML={{ __html: html }} />;\n',
    );
    expect(bad.some((m) => m.message.includes('dangerouslySetInnerHTML'))).toBe(true);

    const good = await lint(
      'export const X = ({ text }: { text: string }) => <div>{text}</div>;\n',
    );
    expect(good).toEqual([]);
  });
});
