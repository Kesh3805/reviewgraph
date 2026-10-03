import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

/** Code that would let ReviewGraph merge or approve (INV-011/INV-012). */
export const FORBIDDEN: { name: string; pattern: RegExp }[] = [
  { name: 'pulls.merge', pattern: /pulls\.merge/ },
  { name: '.merge(', pattern: /\.merge\(/ },
  { name: '/merge', pattern: /\/merge/ },
  { name: 'merge_method', pattern: /merge_method/ },
  { name: 'squash', pattern: /squash/i },
  { name: 'rebase_merge', pattern: /rebase_merge/ },
  { name: 'enablePullRequestAutoMerge', pattern: /enablePullRequestAutoMerge/ },
  { name: 'mergePullRequest', pattern: /mergePullRequest/ },
  { name: "contents: 'write'", pattern: /contents\s*:\s*['"]write['"]/ },
  { name: 'APPROVE event', pattern: /['"]APPROVE['"]/ },
  { name: 'REQUEST_CHANGES event', pattern: /['"]REQUEST_CHANGES['"]/ },
];

/**
 * Source text with every comment blanked out. Comment ranges are collected from the parsed
 * token stream (the parser knows regex literals from division), so strings and code stay intact.
 */
export function stripComments(source: string): string {
  const sf = ts.createSourceFile('scan.ts', source, ts.ScriptTarget.Latest, true);
  const ranges: ts.CommentRange[] = [];
  const visit = (node: ts.Node): void => {
    const children = node.getChildren(sf);
    if (children.length === 0) {
      ranges.push(...(ts.getLeadingCommentRanges(source, node.getFullStart()) ?? []));
      ranges.push(...(ts.getTrailingCommentRanges(source, node.getEnd()) ?? []));
      return;
    }
    children.forEach(visit);
  };
  visit(sf);
  let out = source;
  for (const { pos, end } of ranges) {
    out = out.slice(0, pos) + out.slice(pos, end).replace(/[^\n]/g, ' ') + out.slice(end);
  }
  return out;
}

export function listTsFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return name === 'node_modules' ? [] : listTsFiles(full);
    return name.endsWith('.ts') ? [full] : [];
  });
}

export interface Violation {
  file: string;
  forbidden: string;
}

export function scanForMergeCapability(dir: string): Violation[] {
  const violations: Violation[] = [];
  for (const file of listTsFiles(dir)) {
    const code = stripComments(readFileSync(file, 'utf8'));
    for (const { name, pattern } of FORBIDDEN) {
      if (pattern.test(code)) violations.push({ file, forbidden: name });
    }
  }
  return violations;
}
