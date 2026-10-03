import { normalizeGithubEvent } from '../../../src/providers/github/normalize';
import { fixture } from '../../helpers/fixtures';

const PERMISSIONS = {
  contents: 'read',
  metadata: 'read',
  pull_requests: 'write',
  checks: 'write',
  issues: 'read',
};
const BASE = {
  type: 'installation',
  provider: 'github',
  installationId: '5150',
  account: { login: 'Acme-Corp', kind: 'organization' },
  added: [],
  removed: [],
};

describe('installation lifecycle normalization (pure)', () => {
  it('installation_created_lists_repositories', () => {
    expect(
      normalizeGithubEvent('installation', fixture('installation.created.json'), 'd-1'),
    ).toEqual({
      ...BASE,
      deliveryId: 'd-1',
      kind: 'created',
      permissions: PERMISSIONS,
      added: [
        { providerRepoId: '700001', fullName: 'Acme-Corp/billing', isPrivate: true },
        { providerRepoId: '700002', fullName: 'Acme-Corp/web', isPrivate: false },
      ],
    });
  });

  it.each([
    ['installation.deleted.json', 'deleted'],
    ['installation.suspend.json', 'suspend'],
    ['installation.unsuspend.json', 'unsuspend'],
    ['installation.new_permissions_accepted.json', 'new_permissions_accepted'],
  ])('%s maps to %s without repositories', (file, kind) => {
    const event = normalizeGithubEvent('installation', fixture(file), 'd-2');
    expect(event).toMatchObject({ type: 'installation', kind, installationId: '5150' });
    // Only `created` carries the repository list; the others act on the installation as a whole.
    expect(event).toMatchObject({ added: [], removed: [] });
  });

  it('installation_repositories_added_and_removed', () => {
    expect(
      normalizeGithubEvent(
        'installation_repositories',
        fixture('installation_repositories.added.json'),
        'd-3',
      ),
    ).toMatchObject({
      kind: 'repositories_added',
      added: [{ providerRepoId: '700003', fullName: 'Acme-Corp/infra', isPrivate: true }],
      removed: [],
    });
    expect(
      normalizeGithubEvent(
        'installation_repositories',
        fixture('installation_repositories.removed.json'),
        'd-4',
      ),
    ).toMatchObject({
      kind: 'repositories_removed',
      added: [],
      removed: [{ providerRepoId: '700001', fullName: 'Acme-Corp/billing', isPrivate: true }],
    });
  });

  it('maps account types', () => {
    const user = fixture('installation.created.json');
    user.installation.account.type = 'User';
    expect(normalizeGithubEvent('installation', user, 'd')).toMatchObject({
      account: { kind: 'user' },
    });
  });

  it('ignores unsupported actions and malformed payloads', () => {
    expect(normalizeGithubEvent('installation', { action: 'weird' }, 'd')).toEqual({
      ignored: true,
      reason: 'unsupported_action',
    });
    expect(normalizeGithubEvent('installation', { action: 'created' }, 'd')).toEqual({
      ignored: true,
      reason: 'malformed',
    });
    expect(normalizeGithubEvent('installation', null, 'd')).toEqual({
      ignored: true,
      reason: 'malformed',
    });
    expect(normalizeGithubEvent('installation_repositories', { action: 'moved' }, 'd')).toEqual({
      ignored: true,
      reason: 'unsupported_action',
    });
  });
});
