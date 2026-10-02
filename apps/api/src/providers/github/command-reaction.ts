import { Inject, Injectable } from '@nestjs/common';
import type { ReviewCommandEvent } from '../ports';
import type { GithubAppAuth } from './app-auth.service';
import { toProviderError } from './errors';
import { GITHUB_APP_AUTH } from './github.tokens';

/**
 * Acknowledges an accepted `/review` command with an `eyes` reaction on the comment. No text
 * comment is ever posted in reply (GH-004).
 */
@Injectable()
export class GithubCommandAcknowledger {
  constructor(@Inject(GITHUB_APP_AUTH) private readonly auth: GithubAppAuth | null) {}

  async acknowledge(event: ReviewCommandEvent): Promise<void> {
    if (!this.auth) return;
    const octokit = await this.auth.getOctokit(event.installationId);
    try {
      await octokit.request('POST /repos/{owner}/{repo}/issues/comments/{comment_id}/reactions', {
        owner: event.repo.owner,
        repo: event.repo.name,
        comment_id: Number(event.commentId),
        content: 'eyes',
      });
    } catch (err) {
      throw toProviderError(err, 'command acknowledgement');
    }
  }
}
