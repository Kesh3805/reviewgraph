import type { RenderableFinding } from '../../src/publisher/render/types';

/** The PRD section 58 auth-bypass example as a finding. */
export function authBypass(over: Partial<RenderableFinding> = {}): RenderableFinding {
  return {
    id: 'f-1',
    shortId: 'RG-12-001',
    fingerprint: 'fp-abc123',
    reviewRunId: 'run-77',
    severity: 'high',
    title: 'Authorization check bypassed',
    whatChanged:
      'authorize() now accepts user.role === "admin" directly and no longer calls PermissionService.check().',
    whyRisky:
      'AdminService.updateUser() reaches this path from the user-management endpoint, so resource-level permissions are no longer evaluated for that operation.',
    behaviorResult: 'Any admin can update any user, including users outside their tenant.',
    correctiveDirection: 'Preserve the resource permission check before permitting the mutation.',
    evidencePath: [
      'AuthService.authorize()',
      'AdminService.updateUser()',
      'UserController.update()',
    ],
    reviewer: 'security',
    confidence: 0.91,
    location: { path: 'src/auth.ts', startLine: 12, endLine: 12, side: 'head' },
    ...over,
  };
}

/** One file, two hunks: new lines 10-16 (13-14 added) and 40-44 (41 added); old lines 40-41 deleted. */
export const PATCH = [
  '@@ -10,5 +10,7 @@ function a() {',
  ' ctx10',
  ' ctx11',
  ' ctx12',
  '+add13',
  '+add14',
  ' ctx15',
  ' ctx16',
  '@@ -38,5 +40,4 @@ function b() {',
  ' ctx40',
  '+add41',
  ' ctx42',
  '-del-old-a',
  '-del-old-b',
  ' ctx43',
].join('\n');
