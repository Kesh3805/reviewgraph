import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import {
  APP_EVENTS,
  APP_PERMISSIONS,
  diffPermissions,
  permissionsMatch,
} from '../../src/providers/github/app-manifest';
import { scanForMergeCapability, stripComments } from './scan';

const SRC = resolve(__dirname, '../../src');
const MANIFEST = resolve(__dirname, '../../../../infra/github/app-manifest.json');

describe('no-merge guarantee (INV-011)', () => {
  it('module_exposes_no_merge_capability_ts', () => {
    expect(scanForMergeCapability(SRC)).toEqual([]);
  });

  it('planted_merge_call_fails_test', () => {
    const dir = mkdtempSync(join(tmpdir(), 'rg-no-merge-'));
    try {
      const plant = (name: string, code: string): void => writeFileSync(join(dir, name), code);
      plant('a.ts', 'await octokit.pulls.merge({ owner, repo, pull_number: 1 });\n');
      plant('b.ts', "await octokit.request('PUT /repos/{o}/{r}/pulls/{n}/merge');\n");
      plant('c.ts', "const body = { event: 'APPROVE' };\n");
      plant('d.ts', "const perms = { contents: 'write' };\n");
      plant('e.ts', 'const q = `mutation { enablePullRequestAutoMerge }`;\n');
      const found = scanForMergeCapability(dir);
      expect(new Set(found.map((v) => v.file.split(/[\\/]/).pop()))).toEqual(
        new Set(['a.ts', 'b.ts', 'c.ts', 'd.ts', 'e.ts']),
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('comments are ignored but code after a comment or regex is still scanned', () => {
    const dir = mkdtempSync(join(tmpdir(), 'rg-no-merge-'));
    try {
      writeFileSync(
        join(dir, 'ok.ts'),
        '// octokit.pulls.merge is forbidden\n/* merge_method: squash */\nexport const x = 1;\n',
      );
      writeFileSync(
        join(dir, 'sneaky.ts'),
        'const re = /a\\/b/; const half = 4 / 2; /* c */ octokit.pulls.merge(1); // trailing\n',
      );
      const found = scanForMergeCapability(dir).map((v) => v.file.split(/[\\/]/).pop());
      expect(found).toContain('sneaky.ts');
      expect(found).not.toContain('ok.ts');
      expect(stripComments('a // pulls.merge\nb')).not.toContain('pulls.merge');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('app_manifest_permissions_exact', () => {
    const manifest = JSON.parse(readFileSync(MANIFEST, 'utf8')) as {
      default_permissions: Record<string, string>;
      default_events: string[];
    };
    expect(manifest.default_permissions).toEqual({
      contents: 'read',
      pull_requests: 'write',
      checks: 'write',
      metadata: 'read',
      issues: 'read',
    });
    expect(manifest.default_permissions).toEqual(APP_PERMISSIONS);
    expect(manifest.default_events).toEqual([
      'pull_request',
      'issue_comment',
      'installation',
      'installation_repositories',
    ]);
    expect(manifest.default_events).toEqual([...APP_EVENTS]);
    for (const forbidden of ['administration', 'workflows', 'actions', 'statuses']) {
      expect(manifest.default_permissions).not.toHaveProperty(forbidden);
    }
  });

  it('diffPermissions flags extra, missing and elevated permissions', () => {
    expect(permissionsMatch(diffPermissions({ ...APP_PERMISSIONS }))).toBe(true);
    expect(diffPermissions({ ...APP_PERMISSIONS, contents: 'write' }).mismatched).toEqual([
      'contents:write',
    ]);
    expect(diffPermissions({ ...APP_PERMISSIONS, administration: 'write' }).extra).toEqual([
      'administration:write',
    ]);
    const rest = Object.fromEntries(
      Object.entries(APP_PERMISSIONS).filter(([k]) => k !== 'checks'),
    );
    expect(diffPermissions(rest).missing).toEqual(['checks']);
  });
});
