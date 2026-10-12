import { test } from 'node:test';
import assert from 'node:assert/strict';
import { affected } from './affected-filter.mjs';

const pkgs = [
  { name: '@reviewgraph/contracts', dir: 'packages/contracts', deps: [] },
  { name: '@reviewgraph/api', dir: 'apps/api', deps: ['@reviewgraph/contracts'] },
  { name: '@reviewgraph/web', dir: 'apps/web', deps: ['@reviewgraph/contracts'] },
];

test('affected_filter_selects_dependents_of_contracts', () => {
  const out = affected(['packages/contracts/src/a.ts'], pkgs);
  assert.match(out, /@reviewgraph\/api/);
  assert.match(out, /@reviewgraph\/web/);
  assert.match(out, /@reviewgraph\/contracts/);
});

test('affected_filter_selects_all_on_lockfile_change', () => {
  assert.equal(affected(['pnpm-lock.yaml'], pkgs), 'ALL');
  assert.equal(affected(['packages/config/eslint.mjs'], pkgs), 'ALL');
});

test('affected_filter_empty_for_docs_only_change', () => {
  assert.equal(affected(['docs/x.md'], pkgs), '');
});
