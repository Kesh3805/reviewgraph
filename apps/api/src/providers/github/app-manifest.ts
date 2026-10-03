/**
 * The exact permission set of the ReviewGraph GitHub App (GH-010). It mirrors
 * `infra/github/app-manifest.json` (a test keeps the two identical). There is deliberately no
 * `contents: write`, `administration` or `workflows`: with a read-only contents token, merging
 * is impossible server-side (INV-011).
 */
export const APP_PERMISSIONS: Readonly<Record<string, 'read' | 'write'>> = {
  contents: 'read',
  pull_requests: 'write',
  checks: 'write',
  metadata: 'read',
  issues: 'read',
};

export const APP_EVENTS: readonly string[] = [
  'pull_request',
  'issue_comment',
  'installation',
  'installation_repositories',
];

export interface PermissionDiff {
  /** Granted to the App but not in the manifest (for example `contents: write`). */
  extra: string[];
  /** In the manifest but not granted. */
  missing: string[];
  /** Granted at a different level than the manifest. */
  mismatched: string[];
}

export function diffPermissions(actual: Readonly<Record<string, string>>): PermissionDiff {
  const diff: PermissionDiff = { extra: [], missing: [], mismatched: [] };
  for (const [name, level] of Object.entries(actual)) {
    if (!(name in APP_PERMISSIONS)) diff.extra.push(`${name}:${level}`);
    else if (APP_PERMISSIONS[name] !== level) diff.mismatched.push(`${name}:${level}`);
  }
  for (const name of Object.keys(APP_PERMISSIONS)) if (!(name in actual)) diff.missing.push(name);
  return diff;
}

export const permissionsMatch = (d: PermissionDiff): boolean =>
  d.extra.length === 0 && d.missing.length === 0 && d.mismatched.length === 0;
