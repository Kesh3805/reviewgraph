import { PublisherService } from '../src/publisher/publisher.service';
import { SupersessionService } from '../src/reviews/supersession.service';
import {
  type Harness,
  type SeededPr,
  addedPatch,
  runState,
  seedFindings,
  seedPr,
  seedRun,
  sha,
  startHarness,
} from './github-harness';

const FILE = 'src/billing/invoice.ts';

describe('publisher and stale resolution (integration)', () => {
  let h: Harness;
  let publisher: PublisherService;

  beforeAll(async () => {
    h = await startHarness();
    publisher = h.app.get(PublisherService);
  });
  afterAll(async () => {
    await h.close();
  });
  beforeEach(() => {
    h.jobs.reset();
    h.github.faults = {};
  });

  async function publishingPr(head = sha('a')): Promise<SeededPr> {
    const pr = await seedPr(h, head);
    h.github.setPull(pr.fullName, {
      number: pr.number,
      headSha: head,
      files: [
        { filename: FILE, status: 'modified', additions: 40, deletions: 0, patch: addedPatch(40) },
      ],
    });
    return pr;
  }

  const postsFor = (pr: SeededPr) =>
    h.github
      .reviewPosts()
      .filter((r) => r.path === `/repos/${pr.fullName}/pulls/${pr.number}/reviews`);
  const reviewsFor = (pr: SeededPr) =>
    h.github.reviews.filter((r) => r.repo === pr.fullName && r.pull === pr.number);
  const checkRunsFor = (pr: SeededPr) => h.github.checkRuns.filter((c) => c.repo === pr.fullName);
  const published = (runId: string) =>
    h.admin
      .selectFrom('published_findings')
      .selectAll()
      .where('review_run_id', '=', runId)
      .orderBy('start_line')
      .execute();

  describe('GH-009', () => {
    it('single_review_post_with_all_comments and event_is_comment_constant', async () => {
      const pr = await publishingPr();
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await seedFindings(h, pr, run, [
        { path: FILE, line: 3 },
        { path: FILE, line: 7, severity: 'medium' },
      ]);
      await expect(publisher.publish(run)).resolves.toBe('published');

      const posts = postsFor(pr);
      expect(posts).toHaveLength(1);
      const body = posts[0]!.body as {
        event: string;
        commit_id: string;
        body: string;
        comments: { path: string; line: number; side: string; body: string }[];
      };
      expect(body.event).toBe('COMMENT');
      expect(body.commit_id).toBe(sha('a'));
      expect(body.body).toContain(`reviewgraph:run=${run}`);
      expect(body.body).toContain('No merge performed');
      expect(body.comments.map((c) => [c.path, c.line, c.side])).toEqual([
        [FILE, 3, 'RIGHT'],
        [FILE, 7, 'RIGHT'],
      ]);
      expect(await runState(h, run)).toBe('COMPLETED');
      const pub = await h.admin
        .selectFrom('publications')
        .selectAll()
        .where('review_run_id', '=', run)
        .executeTakeFirstOrThrow();
      expect(pub).toMatchObject({
        state: 'posted',
        provider_review_id: String(reviewsFor(pr)[0]!.id),
      });
    });

    it('published_findings_mapped_by_marker', async () => {
      const pr = await publishingPr();
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      const { verifiedIds } = await seedFindings(h, pr, run, [
        { path: FILE, line: 3 },
        { path: FILE, line: 9 },
      ]);
      await publisher.publish(run);
      const review = reviewsFor(pr)[0]!;
      const rows = await published(run);
      expect(rows).toHaveLength(2);
      for (const [i, row] of rows.entries()) {
        expect(row.verified_finding_id).toBe(verifiedIds[i]);
        const comment = review.comments.find((c) => String(c.id) === row.provider_comment_id)!;
        expect(comment.line).toBe(row.end_line);
        expect(row).toMatchObject({
          placement: 'inline',
          path: FILE,
          side: 'RIGHT',
          status: 'open',
          provider_review_id: String(review.id),
          head_sha: sha('a'),
        });
      }
    });

    it('retry_after_crash_adopts_existing_review', async () => {
      const pr = await publishingPr();
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await seedFindings(h, pr, run, [{ path: FILE, line: 5 }]);
      h.github.faults.reviewPost = 'drop_after_write';
      // GitHub accepted the POST, then the connection died: the job fails and is retried.
      await expect(publisher.publish(run)).rejects.toMatchObject({ kind: 'transient' });
      expect(reviewsFor(pr)).toHaveLength(1);
      await expect(publisher.publish(run)).resolves.toBe('adopted');
      expect(postsFor(pr)).toHaveLength(1);
      expect(reviewsFor(pr)).toHaveLength(1);
      const rows = await published(run);
      expect(rows[0]?.provider_comment_id).toBe(String(reviewsFor(pr)[0]!.comments[0]!.id));
      expect(await runState(h, run)).toBe('COMPLETED');
      // A further retry is a no-op.
      await expect(publisher.publish(run)).resolves.toBe('already_published');
      expect(postsFor(pr)).toHaveLength(1);
    });

    it('422_relocates_to_summary_and_retries_once', async () => {
      const pr = await publishingPr();
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await seedFindings(h, pr, run, [{ path: FILE, line: 4, title: 'Double charge' }]);
      h.github.faults.reviewPost = 422;
      await expect(publisher.publish(run)).resolves.toBe('published');
      const posts = postsFor(pr);
      expect(posts).toHaveLength(2);
      const second = posts[1]!.body as { comments: unknown[]; body: string };
      expect(second.comments).toEqual([]);
      expect(second.body).toContain('Findings outside the diff');
      expect(second.body).toContain('Double charge');
      const rows = await published(run);
      expect(rows.map((r) => [r.placement, r.provider_comment_id])).toEqual([['summary', null]]);
    });

    it('check_run_neutral_with_findings', async () => {
      const pr = await publishingPr();
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await seedFindings(h, pr, run, [{ path: FILE, line: 2 }]);
      await publisher.publish(run);
      const [check] = checkRunsFor(pr);
      expect(check?.body).toMatchObject({
        name: 'ReviewGraph',
        head_sha: sha('a'),
        status: 'completed',
        conclusion: 'neutral',
        external_id: run,
      });
      const row = await h.admin
        .selectFrom('check_runs')
        .selectAll()
        .where('review_run_id', '=', run)
        .executeTakeFirstOrThrow();
      expect(row.provider_check_run_id).toBe(String(check!.id));
    });

    it('check_run_success_only_when_clean_and_complete', async () => {
      const clean = await publishingPr();
      const cleanRun = await seedRun(h, clean, 'PUBLISHING', sha('a'));
      await seedFindings(h, clean, cleanRun, []);
      await publisher.publish(cleanRun);
      expect(checkRunsFor(clean)[0]?.body).toMatchObject({ conclusion: 'success' });

      const degraded = await publishingPr();
      const degradedRun = await seedRun(h, degraded, 'PUBLISHING', sha('a'));
      await h.admin
        .insertInto('reviewer_runs')
        .values({
          organization_id: degraded.org.organizationId,
          review_run_id: degradedRun,
          reviewer: 'security',
          state: 'failed',
          error_class: 'transient',
        })
        .execute();
      await publisher.publish(degradedRun);
      expect(checkRunsFor(degraded)[0]?.body).toMatchObject({
        conclusion: 'neutral',
        output: { title: 'Review incomplete' },
      });
      expect((postsFor(degraded)[0]!.body as { body: string }).body).toContain(
        'Review completed with reduced coverage',
      );
    });

    it('check_run_never_success_on_failure', async () => {
      const pr = await publishingPr();
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await seedFindings(h, pr, run, []);
      h.github.faults.reviewPost = 403;
      await expect(publisher.publish(run)).resolves.toBe('failed_permanent');
      expect(await runState(h, run)).toBe('FAILED_PUBLISH');
      expect(checkRunsFor(pr)[0]?.body).toMatchObject({
        conclusion: 'neutral',
        output: { title: 'Review could not be published' },
      });
    });

    it('superseded_at_gate_posts_nothing', async () => {
      const pr = await publishingPr();
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await seedFindings(h, pr, run, [{ path: FILE, line: 3 }]);
      await h.app.get(SupersessionService).startReview({
        organizationId: pr.org.organizationId,
        pullRequestId: pr.pullRequestId,
        headSha: sha('b'),
        baseSha: sha('0'),
        trigger: 'webhook',
      });
      await expect(publisher.publish(run)).resolves.toBe('skipped_superseded');
      expect(postsFor(pr)).toHaveLength(0);
      expect(await runState(h, run)).toBe('SUPERSEDED');
    });
  });

  describe('GH-011', () => {
    /** Publishes run 1 at head A with one finding, then prepares run 2 at head B. */
    async function twoHeads(opts: { changed: string[] | null; secondFindings: 'same' | 'none' }) {
      const pr = await publishingPr(sha('a'));
      const run1 = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      const first = await seedFindings(h, pr, run1, [{ path: FILE, line: 6 }]);
      await publisher.publish(run1);
      const [row1] = await published(run1);

      await h.admin
        .updateTable('pull_requests')
        .set({ head_sha: sha('b') })
        .where('id', '=', pr.pullRequestId)
        .execute();
      h.github.setPull(pr.fullName, { number: pr.number, headSha: sha('b') });
      if (opts.changed)
        h.github.compares.set(`${pr.fullName}:${sha('a')}...${sha('b')}`, opts.changed);
      const run2 = await seedRun(h, pr, 'PUBLISHING', sha('b'));
      await seedFindings(
        h,
        pr,
        run2,
        opts.secondFindings === 'same'
          ? [{ path: FILE, line: 6, fingerprint: first.fingerprints[0] }]
          : [],
      );
      return { pr, run1, run2, row1: row1! };
    }

    const statusOf = async (id: string) =>
      h.admin
        .selectFrom('published_findings')
        .select(['status', 'resolved_in_run_id'])
        .where('id', '=', id)
        .executeTakeFirstOrThrow();

    it('fixed_finding_thread_resolved', async () => {
      const { pr, run2, row1 } = await twoHeads({ changed: [FILE], secondFindings: 'none' });
      const before = h.github.resolvedThreadIds.length;
      await expect(publisher.publish(run2)).resolves.toBe('published');
      expect(h.github.resolvedThreadIds.slice(before)).toEqual([`T_${row1.provider_comment_id}`]);
      expect(await statusOf(row1.id)).toEqual({ status: 'resolved', resolved_in_run_id: run2 });
      expect(postsFor(pr)).toHaveLength(2);
    });

    it('still_present_not_reposted', async () => {
      const { pr, run2, row1 } = await twoHeads({ changed: [FILE], secondFindings: 'same' });
      await publisher.publish(run2);
      const second = postsFor(pr)[1]!.body as { comments: unknown[] };
      expect(second.comments).toEqual([]);
      expect((await statusOf(row1.id)).status).toBe('carried_over');
      expect(await published(run2)).toEqual([]);
    });

    it('unknown_left_open_listed_in_summary', async () => {
      const { pr, run2, row1 } = await twoHeads({ changed: [], secondFindings: 'none' });
      const before = h.github.resolvedThreadIds.length;
      await publisher.publish(run2);
      expect(h.github.resolvedThreadIds.length).toBe(before);
      expect((await statusOf(row1.id)).status).toBe('unknown');
      expect((postsFor(pr)[1]!.body as { body: string }).body).toContain('Previously reported');
    });

    it('human_thread_never_resolved', async () => {
      const { pr, run2, row1 } = await twoHeads({ changed: [FILE], secondFindings: 'none' });
      // The tracked comment's thread was started by a human (not the App).
      h.github.addHumanThread(pr.fullName, pr.number, 987_654_321);
      await h.admin
        .updateTable('published_findings')
        .set({ provider_comment_id: '987654321' })
        .where('id', '=', row1.id)
        .execute();
      const before = h.github.resolvedThreadIds.length;
      await publisher.publish(run2);
      expect(h.github.resolvedThreadIds.length).toBe(before);
      expect((await statusOf(row1.id)).status).toBe('open');
    });

    it('graphql_failure_does_not_fail_publish', async () => {
      const { run2, row1 } = await twoHeads({ changed: [FILE], secondFindings: 'none' });
      h.github.faults.graphql = 400;
      await expect(publisher.publish(run2)).resolves.toBe('published');
      expect(await runState(h, run2)).toBe('COMPLETED');
      expect((await statusOf(row1.id)).status).toBe('open');
    });
  });
});
