import { randomBytes } from 'node:crypto';
import { Inject, Injectable } from '@nestjs/common';
import { SignJWT, jwtVerify } from 'jose';
import type { Redis } from 'ioredis';
import { sql } from 'kysely';
import { REDIS } from '../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { DbService } from '../db/db.module';
import { MembershipService, type UserMembership } from '../tenancy/membership.service';
import { GithubOAuthClient, pkceChallenge } from './github-oauth.client';

export const OAUTH_STATE_TTL_SECONDS = 600;
const STATE_AUDIENCE = 'rg-oauth-state';
export const oauthStateKey = (state: string): string => `rg:oauth:state:${state}`;

export interface LoginStart {
  /** Where to send the browser. */
  authorizeUrl: string;
  /** Value of the signed `rg_oauth_state` cookie. */
  stateCookie: string;
}

export interface LoginResult {
  userId: string;
}

export interface UserProfile {
  id: string;
  login: string;
  displayName: string | null;
  avatarUrl: string | null;
}

@Injectable()
export class AuthService {
  private readonly key: Uint8Array;

  constructor(
    @Inject(APP_CONFIG) config: AppConfig,
    @Inject(REDIS) private readonly redis: Redis,
    private readonly dbs: DbService,
    private readonly oauth: GithubOAuthClient,
    private readonly memberships: MembershipService,
  ) {
    this.key = new TextEncoder().encode(config.SESSION_JWT_SECRET);
  }

  /**
   * Starts the flow: a random 32-byte `state` (single use, kept in Redis with the PKCE verifier)
   * and a short-lived signed cookie that binds the state to this browser.
   */
  async start(): Promise<LoginStart> {
    const state = randomBytes(32).toString('base64url');
    const verifier = randomBytes(32).toString('base64url');
    await this.redis.set(oauthStateKey(state), verifier, 'EX', OAUTH_STATE_TTL_SECONDS);
    const stateCookie = await new SignJWT({ st: state })
      .setProtectedHeader({ alg: 'HS256' })
      .setAudience(STATE_AUDIENCE)
      .setIssuedAt()
      .setExpirationTime(`${OAUTH_STATE_TTL_SECONDS}s`)
      .sign(this.key);
    return { authorizeUrl: this.oauth.authorizeUrl(state, pkceChallenge(verifier)), stateCookie };
  }

  /**
   * Checks the callback `state` against the cookie and consumes it (Redis GETDEL, so a replay or
   * a concurrent second callback gets nothing). Returns the PKCE verifier, or null when the
   * state is missing, forged, expired or already used.
   */
  async consumeState(
    cookieValue: string | undefined,
    queryState: string | undefined,
  ): Promise<string | null> {
    if (!cookieValue || !queryState) return null;
    let cookieState: unknown;
    try {
      const { payload } = await jwtVerify(cookieValue, this.key, {
        algorithms: ['HS256'],
        audience: STATE_AUDIENCE,
      });
      cookieState = payload.st;
    } catch {
      return null;
    }
    if (typeof cookieState !== 'string' || cookieState !== queryState) return null;
    return this.redis.getdel(oauthStateKey(queryState));
  }

  /**
   * Exchanges the code, upserts the user and refreshes memberships from the installations the
   * user can access. The OAuth token is used for these two reads only and is never stored.
   */
  async completeLogin(code: string, verifier: string): Promise<LoginResult> {
    const token = await this.oauth.exchangeCode(code, verifier);
    const [ghUser, installations] = await Promise.all([
      this.oauth.getUser(token),
      this.oauth.listInstallations(token),
    ]);

    const user = await this.dbs.withTx(null, (trx) =>
      trx
        .insertInto('users')
        .values({
          provider: 'github',
          provider_user_id: String(ghUser.id),
          login: ghUser.login,
          display_name: ghUser.name,
          email: ghUser.email,
          avatar_url: ghUser.avatarUrl,
        })
        .onConflict((oc) =>
          oc.columns(['provider', 'provider_user_id']).doUpdateSet({
            login: ghUser.login,
            display_name: ghUser.name,
            email: ghUser.email,
            avatar_url: ghUser.avatarUrl,
          }),
        )
        .returning('id')
        .executeTakeFirstOrThrow(),
    );

    // Only installations already known to the control plane (recorded by the installation
    // webhook, GH-013) grant access. A personal installation makes its owner `owner`; an
    // organization installation grants `member`, and an admin/owner role assigned in the
    // dashboard is never downgraded by the sync.
    const roleByOrg = new Map<string, 'owner' | 'member'>();
    for (const inst of installations) {
      const orgId = await this.memberships.installationOrg('github', inst.id);
      if (!orgId) continue;
      const personal =
        inst.accountType === 'User' &&
        inst.accountLogin.toLowerCase() === ghUser.login.toLowerCase();
      if (personal || !roleByOrg.has(orgId)) roleByOrg.set(orgId, personal ? 'owner' : 'member');
    }
    const orgIds = [...roleByOrg.keys()];
    const roles = orgIds.map((id) => roleByOrg.get(id)!);
    await this.dbs.withTx(null, (trx) =>
      sql`select rg_sync_user_memberships(${user.id}::uuid, ${orgIds}::uuid[], ${roles}::text[])`.execute(
        trx,
      ),
    );
    return { userId: user.id };
  }

  async profile(userId: string): Promise<UserProfile | null> {
    const row = await this.dbs.withTx(null, (trx) =>
      trx
        .selectFrom('users')
        .select(['id', 'login', 'display_name', 'avatar_url'])
        .where('id', '=', userId)
        .executeTakeFirst(),
    );
    return row
      ? { id: row.id, login: row.login, displayName: row.display_name, avatarUrl: row.avatar_url }
      : null;
  }

  organizations(userId: string): Promise<UserMembership[]> {
    return this.memberships.listForUser(userId);
  }
}

/** Random token for the double-submit cookie. */
export function newCsrfToken(): string {
  return randomBytes(32).toString('base64url');
}
