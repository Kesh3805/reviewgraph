import { Inject, Injectable } from '@nestjs/common';
import { ProviderError, type ActorPermission, type RepoRef } from '../ports';
import type { GithubAppAuth } from './app-auth.service';
import { toProviderError } from './errors';
import { GITHUB_APP_AUTH } from './github.tokens';

/** Looks up a user's permission on a repository (needed to authorize `/review` commands). */
export interface ActorPermissionLookup {
  getActorPermission(ref: RepoRef, login: string): Promise<ActorPermission>;
}
export const ACTOR_PERMISSION_LOOKUP = Symbol('ACTOR_PERMISSION_LOOKUP');

interface PermissionResponse {
  permission?: string;
  role_name?: string;
}

/** `maintain` collapses to write and `triage` to read: only write/admin may command reviews. */
export function mapGithubPermission(data: PermissionResponse): ActorPermission {
  const role = data.role_name ?? data.permission;
  switch (role) {
    case 'admin':
      return 'admin';
    case 'maintain':
    case 'write':
      return 'write';
    case 'triage':
    case 'read':
      return 'read';
    default:
      return data.permission === 'admin' ? 'admin' : data.permission === 'write' ? 'write' : 'none';
  }
}

@Injectable()
export class GithubActorPermissions implements ActorPermissionLookup {
  constructor(@Inject(GITHUB_APP_AUTH) private readonly auth: GithubAppAuth | null) {}

  async getActorPermission(ref: RepoRef, login: string): Promise<ActorPermission> {
    if (!this.auth) throw new ProviderError('forbidden', 'GitHub integration is disabled');
    const octokit = await this.auth.getOctokit(ref.installationId);
    try {
      const res = await octokit.request(
        'GET /repos/{owner}/{repo}/collaborators/{username}/permission',
        { owner: ref.owner, repo: ref.name, username: login },
      );
      return mapGithubPermission(res.data as PermissionResponse);
    } catch (err) {
      throw toProviderError(err, 'actor permission lookup');
    }
  }
}
