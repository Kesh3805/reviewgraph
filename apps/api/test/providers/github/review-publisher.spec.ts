import RedisMock from 'ioredis-mock';
import { GithubAppAuth } from '../../../src/providers/github/app-auth.service';
import { isAppAuthor } from '../../../src/providers/github/graphql';
import {
  GithubReviewPublisher,
  hasMarker,
  parseFindingMarker,
} from '../../../src/providers/github/review-publisher';
import { InstallationTokenCache } from '../../../src/providers/github/token-cache';
import type { PrRef, PublishRequest } from '../../../src/providers/ports';
import { testConfig } from '../../helpers';
import { generateAppKey } from '../../helpers/fake-github';
import { FakeGithubApi } from '../../helpers/fake-github-api';

const KEY = generateAppKey();
const PR: PrRef = {
  provider: 'github',
  installationId: '9',
  owner: 'acme',
  name: 'billing',
  number: 5,
};
const RUN = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2c';

const comment = (fp: string, line: number) => ({
  path: 'src/a.ts',
  line,
  side: 'RIGHT' as const,
  body: `text\n<!-- reviewgraph:finding=${fp} run=${RUN} -->`,
  findingId: `vf-${line}`,
});

describe('GithubReviewPublisher (GH-009, GH-011)', () => {
  let github: FakeGithubApi;
  let publisher: GithubReviewPublisher;

  beforeEach(async () => {
    github = new FakeGithubApi(KEY.publicKey);
    await github.start();
    const auth = new GithubAppAuth({
      appId: '1',
      privateKey: KEY.privateKey,
      apiUrl: github.url,
      cache: new InstallationTokenCache(new RedisMock(), Buffer.alloc(32, 5)),
      retries: false,
    });
    publisher = new GithubReviewPublisher(
      testConfig({ GITHUB_API_URL: github.url, GITHUB_APP_SLUG: 'reviewgraph' }),
      auth,
    );
  });
  afterEach(async () => {
    await github.stop();
  });

  const req = (comments = [comment('v1:aa', 3), comment('v1:bb', 8)]): PublishRequest => ({
    pr: PR,
    headSha: 'c'.repeat(40),
    event: 'COMMENT',
    summary: `## ReviewGraph\n\n<!-- reviewgraph:run=${RUN} head=${'c'.repeat(40)} -->`,
    comments,
    marker: `reviewgraph:run=${RUN}`,
  });

  it('single_review_post_with_all_comments; event_is_comment_constant', async () => {
    const result = await publisher.publishReview(req());
    const posts = github.reviewPosts();
    expect(posts).toHaveLength(1);
    const body = posts[0]!.body as { event: string; comments: unknown[]; commit_id: string };
    expect(body.event).toBe('COMMENT');
    expect(body.commit_id).toBe('c'.repeat(40));
    expect(body.comments).toHaveLength(2);
    // Comment ids map back to our finding ids through the markers.
    expect(result.comments.map((c) => c.findingId)).toEqual(['vf-3', 'vf-8']);
    expect(result.providerReviewId).toBe(String(github.reviews[0]!.id));
  });

  it('refuses any event but COMMENT even through a cast', async () => {
    const bad = { ...req(), event: 'APPROVE' as unknown as 'COMMENT' };
    await expect(publisher.publishReview(bad)).rejects.toMatchObject({ kind: 'invalid' });
    expect(github.reviewPosts()).toHaveLength(0);
  });

  it('findExistingReview adopts the review carrying the run marker', async () => {
    await expect(publisher.findExistingReview(PR, `reviewgraph:run=${RUN}`)).resolves.toBeNull();
    await publisher.publishReview(req());
    const found = await publisher.findExistingReview(PR, `reviewgraph:run=${RUN}`);
    expect(found?.providerReviewId).toBe(String(github.reviews[0]!.id));
    expect(found?.comments.map((c) => c.findingId)).toEqual(['v1:aa', 'v1:bb']);
    // A different run's marker (a prefix of nothing) does not match.
    await expect(publisher.findExistingReview(PR, 'reviewgraph:run=0197')).resolves.toBeNull();
  });

  it('fixed_finding_thread_resolved and human_thread_never_resolved', async () => {
    const result = await publisher.publishReview(req());
    github.addHumanThread('acme/billing', 5, 555);
    const ids = [...result.comments.map((c) => c.providerCommentId), '555', '999'];
    const resolved = await publisher.resolveThreads(PR, ids);
    expect(resolved.resolved).toEqual(result.comments.map((c) => c.providerCommentId));
    expect(resolved.skipped).toEqual(['555', '999']);
    expect(github.resolvedThreadIds).toEqual(
      result.comments.map((c) => `T_${c.providerCommentId}`),
    );
    expect(github.threads.find((t) => t.firstCommentId === 555)?.isResolved).toBe(false);
  });

  it('graphql errors surface as ProviderError', async () => {
    github.faults.graphql = 400;
    await expect(publisher.resolveThreads(PR, ['1'])).rejects.toMatchObject({
      name: 'ProviderError',
    });
  });

  it('check runs are created then updated, never with a blocking conclusion', async () => {
    const created = await publisher.upsertCheckRun({
      repo: PR,
      headSha: 'c'.repeat(40),
      name: 'ReviewGraph',
      status: 'completed',
      conclusion: 'neutral',
      title: 't',
      summary: 's',
      externalId: RUN,
    });
    const updated = await publisher.upsertCheckRun({
      repo: PR,
      headSha: 'c'.repeat(40),
      name: 'ReviewGraph',
      status: 'completed',
      conclusion: 'success',
      title: 't2',
      summary: 's2',
      checkRunId: created.checkRunId,
    });
    expect(updated.checkRunId).toBe(created.checkRunId);
    expect(github.checkRuns).toHaveLength(1);
    expect(github.checkRuns[0]!.body).toMatchObject({ conclusion: 'success', external_id: RUN });
  });

  it('marker helpers', () => {
    expect(parseFindingMarker(`x <!-- reviewgraph:finding=v1:ab run=${RUN} -->`)).toEqual({
      finding: 'v1:ab',
      run: RUN,
    });
    expect(parseFindingMarker('no marker')).toBeNull();
    expect(hasMarker(`<!-- reviewgraph:run=${RUN} head=x -->`, `reviewgraph:run=${RUN}`)).toBe(
      true,
    );
    expect(hasMarker(`<!-- reviewgraph:run=${RUN}0 -->`, `reviewgraph:run=${RUN}`)).toBe(false);
    expect(isAppAuthor('reviewgraph', 'reviewgraph')).toBe(true);
    expect(isAppAuthor('reviewgraph[bot]', 'reviewgraph')).toBe(true);
    expect(isAppAuthor('octo-dev', 'reviewgraph')).toBe(false);
    expect(isAppAuthor('reviewgraph', undefined)).toBe(false);
  });
});
