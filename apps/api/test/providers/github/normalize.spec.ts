import { branchMatches, isBotLogin, preReviewGuard } from '../../../src/providers/github/guards';
import { REVIEW_COMMAND, normalizeGithubEvent } from '../../../src/providers/github/normalize';
import { fixture } from '../../helpers/fixtures';
import { DEFAULT_REPOSITORY_SETTINGS } from '../../../src/repositories/repository-settings.port';

const REPO = { provider: 'github', installationId: '4242', owner: 'acme', name: 'billing' };
const PR = { ...REPO, number: 1347 };

describe('GitHub event normalization (pure)', () => {
  it('opened_normalized', () => {
    expect(
      normalizeGithubEvent('pull_request', fixture('pull_request.opened.json'), 'd-1'),
    ).toEqual({
      type: 'pull_request_head',
      kind: 'opened',
      provider: 'github',
      deliveryId: 'd-1',
      installationId: '4242',
      repo: REPO,
      pr: PR,
      headSha: 'ec26c3e57ca3a959ca5aad62de7213c562f8c821',
      baseSha: '6dcb09b5b57875f334f61aebed695e2e4193db5e',
      baseRef: 'main',
      author: { login: 'octocat', isBot: false },
      draft: false,
      requestedReviewer: undefined,
    });
  });

  it('synchronize_normalized_with_new_head', () => {
    const event = normalizeGithubEvent(
      'pull_request',
      fixture('pull_request.synchronize.json'),
      'd-2',
    );
    expect(event).toMatchObject({
      type: 'pull_request_head',
      kind: 'synchronize',
      headSha: '9049f1265b7d61be4a8904a9a27120d2064dab3b',
      baseSha: '6dcb09b5b57875f334f61aebed695e2e4193db5e',
    });
  });

  it.each(['reopened', 'ready_for_review'])('%s is a head event', (action) => {
    const payload = { ...fixture('pull_request.opened.json'), action };
    expect(normalizeGithubEvent('pull_request', payload, 'd')).toMatchObject({
      type: 'pull_request_head',
      kind: action,
    });
  });

  it('review_requested_other_user_ignored (and requires the App bot)', () => {
    const payload = fixture('pull_request.review_requested.json');
    const ctx = { botLogin: 'reviewgraph[bot]' };
    expect(normalizeGithubEvent('pull_request', payload, 'd', ctx)).toMatchObject({
      type: 'pull_request_head',
      kind: 'review_requested',
      requestedReviewer: 'reviewgraph[bot]',
    });
    // Case-insensitive on the login.
    payload.requested_reviewer.login = 'ReviewGraph[bot]';
    expect(normalizeGithubEvent('pull_request', payload, 'd', ctx)).toMatchObject({
      kind: 'review_requested',
    });
    payload.requested_reviewer = { login: 'some-human', type: 'User' };
    expect(normalizeGithubEvent('pull_request', payload, 'd', ctx)).toEqual({
      ignored: true,
      reason: 'reviewer_not_app',
    });
    // No configured bot: nothing counts.
    expect(
      normalizeGithubEvent('pull_request', fixture('pull_request.review_requested.json'), 'd'),
    ).toEqual({
      ignored: true,
      reason: 'reviewer_not_app',
    });
    // A team request has no requested_reviewer user.
    delete payload.requested_reviewer;
    expect(normalizeGithubEvent('pull_request', payload, 'd', ctx)).toEqual({
      ignored: true,
      reason: 'reviewer_not_app',
    });
  });

  it('closed_cancels_runs', () => {
    expect(
      normalizeGithubEvent('pull_request', fixture('pull_request.closed.json'), 'd-3'),
    ).toEqual({
      type: 'pull_request_closed',
      provider: 'github',
      deliveryId: 'd-3',
      installationId: '4242',
      repo: REPO,
      pr: PR,
      merged: true,
    });
  });

  it('ignores other pull_request actions and other events', () => {
    for (const action of ['labeled', 'edited', 'assigned', 'converted_to_draft']) {
      const payload = { ...fixture('pull_request.opened.json'), action };
      expect(normalizeGithubEvent('pull_request', payload, 'd')).toEqual({
        ignored: true,
        reason: 'unsupported_action',
      });
    }
    expect(normalizeGithubEvent('push', {}, 'd')).toEqual({
      ignored: true,
      reason: 'unsupported_event',
    });
  });

  it('malformed_payload_ignored', () => {
    const cases: unknown[] = [null, 'text', {}, { action: 'opened' }, { action: 42 }];
    for (const payload of cases) {
      expect(normalizeGithubEvent('pull_request', payload, 'd')).toEqual({
        ignored: true,
        reason: 'malformed',
      });
    }
    const missingHead = fixture('pull_request.opened.json');
    delete missingHead.pull_request.head;
    expect(normalizeGithubEvent('pull_request', missingHead, 'd')).toEqual({
      ignored: true,
      reason: 'malformed',
    });
    const noInstallation = fixture('pull_request.opened.json');
    delete noInstallation.installation;
    expect(normalizeGithubEvent('pull_request', noInstallation, 'd')).toEqual({
      ignored: true,
      reason: 'malformed',
    });
    expect(normalizeGithubEvent('issue_comment', { action: 'created' }, 'd')).toEqual({
      ignored: true,
      reason: 'malformed',
    });
  });

  describe('issue_comment commands', () => {
    const commentWith = (body: string) => {
      const payload = fixture('issue_comment.created.json');
      payload.comment.body = body;
      return payload;
    };

    it('parses /review, /review full and /review cancel into command candidates', () => {
      const base = {
        candidate: 'review_command',
        deliveryId: 'd',
        repo: REPO,
        pr: PR,
        commentId: '9001',
        actor: { login: 'maintainer', isBot: false },
        prAuthor: { login: 'octocat', isBot: false },
        draft: false,
      };
      expect(normalizeGithubEvent('issue_comment', commentWith('/review'), 'd')).toEqual({
        ...base,
        command: 'review',
      });
      expect(normalizeGithubEvent('issue_comment', commentWith('/review full'), 'd')).toEqual({
        ...base,
        command: 'full',
      });
      expect(
        normalizeGithubEvent('issue_comment', commentWith('/review   cancel \r\n'), 'd'),
      ).toEqual({
        ...base,
        command: 'cancel',
      });
    });

    it('ignores text that is not exactly a command', () => {
      for (const body of [
        'please /review',
        '/reviewer',
        '/review fullest',
        '/review full please',
        ' /review',
        'LGTM',
        '/review\nand more',
      ]) {
        expect(normalizeGithubEvent('issue_comment', commentWith(body), 'd')).toEqual({
          ignored: true,
          reason: 'not_a_command',
        });
      }
      expect(REVIEW_COMMAND.test('/review')).toBe(true);
    });

    it('ignores comments on plain issues, edited comments and bot commenters', () => {
      expect(
        normalizeGithubEvent('issue_comment', fixture('issue_comment.on_issue.json'), 'd'),
      ).toEqual({
        ignored: true,
        reason: 'not_a_pull_request_comment',
      });
      const edited = { ...fixture('issue_comment.created.json'), action: 'edited' };
      expect(normalizeGithubEvent('issue_comment', edited, 'd')).toEqual({
        ignored: true,
        reason: 'unsupported_action',
      });
      const bot = fixture('issue_comment.created.json');
      bot.comment.user = { login: 'reviewgraph[bot]', type: 'Bot' };
      expect(normalizeGithubEvent('issue_comment', bot, 'd')).toEqual({
        ignored: true,
        reason: 'bot_author',
      });
    });
  });
});

describe('pre-review guards (ported from github.rs:305-347)', () => {
  const head = (over: Partial<Parameters<typeof preReviewGuard>[0]> = {}) => ({
    author: { login: 'someone', isBot: false },
    draft: false,
    baseRef: 'dev',
    ...over,
  });

  it('draft_skipped', () => {
    expect(preReviewGuard(head({ draft: true }), DEFAULT_REPOSITORY_SETTINGS)).toEqual({
      ignored: true,
      reason: 'draft',
    });
    // Disabled by repository settings.
    expect(
      preReviewGuard(head({ draft: true }), { ...DEFAULT_REPOSITORY_SETTINGS, skipDrafts: false }),
    ).toBeNull();
  });

  it('draft_with_explicit_command_reviewed', () => {
    expect(
      preReviewGuard(head({ draft: true }), DEFAULT_REPOSITORY_SETTINGS, { explicit: true }),
    ).toBeNull();
  });

  it('bot_author_skipped', () => {
    for (const login of ['dependabot[bot]', 'renovate[bot]', 'dependabot', 'Dependabot']) {
      expect(isBotLogin(login)).toBe(true);
      expect(
        preReviewGuard(head({ author: { login, isBot: false } }), DEFAULT_REPOSITORY_SETTINGS),
      ).toEqual({ ignored: true, reason: 'bot_author' });
    }
    expect(
      preReviewGuard(head({ author: { login: 'x', isBot: true } }), DEFAULT_REPOSITORY_SETTINGS),
    ).toEqual({
      ignored: true,
      reason: 'bot_author',
    });
    expect(isBotLogin('botany')).toBe(false);
    expect(
      preReviewGuard(head({ author: { login: 'x[bot]', isBot: true } }), {
        ...DEFAULT_REPOSITORY_SETTINGS,
        skipBots: false,
      }),
    ).toBeNull();
  });

  it('branch_glob_release_star', () => {
    const settings = { ...DEFAULT_REPOSITORY_SETTINGS, targetBranches: ['dev', 'release/*'] };
    expect(preReviewGuard(head({ baseRef: 'release/2.1' }), settings)).toBeNull();
    expect(preReviewGuard(head({ baseRef: 'dev' }), settings)).toBeNull();
    expect(preReviewGuard(head({ baseRef: 'feature/z' }), settings)).toEqual({
      ignored: true,
      reason: 'branch_not_targeted',
    });
    expect(preReviewGuard(head({ baseRef: 'development' }), settings)).toEqual({
      ignored: true,
      reason: 'branch_not_targeted',
    });
    expect(branchMatches('main', [])).toBe(false);
    // An empty list means every branch.
    expect(preReviewGuard(head({ baseRef: 'anything' }), DEFAULT_REPOSITORY_SETTINGS)).toBeNull();
  });

  it('a disabled repository is never reviewed', () => {
    expect(preReviewGuard(head(), { ...DEFAULT_REPOSITORY_SETTINGS, enabled: false })).toEqual({
      ignored: true,
      reason: 'repository_disabled',
    });
  });
});
