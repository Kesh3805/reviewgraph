import { createHash } from 'node:crypto';
import { Inject, Injectable, Logger } from '@nestjs/common';
import { SignJWT, jwtVerify } from 'jose';
import type { Redis } from 'ioredis';
import { sql } from 'kysely';
import { REDIS } from '../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { DbService } from '../db/db.module';
import type { RequestUser } from '../tenancy/request-context';
import { SESSION_TTL_SECONDS } from './cookies';

/** A verified session is trusted from Redis for this long before the database is asked again. */
export const SESSION_CACHE_SECONDS = 60;
export const sessionCacheKey = (sessionId: string): string => `rg:sess:${sessionId}`;

export interface CreatedSession {
  sessionId: string;
  token: string;
  expiresAt: Date;
}

/**
 * Revocable sessions (API-004): the cookie holds a JWT `{sub, sid, iat, exp}` that references a
 * `sessions` row. The row is checked (`revoked_at IS NULL AND expires_at > now()`), with the
 * positive result cached in Redis for 60 s; logout deletes the cache entry so it takes effect at
 * once on this node set.
 */
@Injectable()
export class SessionService {
  private readonly logger = new Logger(SessionService.name);
  private readonly key: Uint8Array;

  constructor(
    @Inject(APP_CONFIG) config: AppConfig,
    @Inject(REDIS) private readonly redis: Redis,
    private readonly dbs: DbService,
  ) {
    this.key = new TextEncoder().encode(config.SESSION_JWT_SECRET);
  }

  async create(userId: string, userAgent: string | undefined): Promise<CreatedSession> {
    const expiresAt = new Date(Date.now() + SESSION_TTL_SECONDS * 1000);
    const uaHash = userAgent ? createHash('sha256').update(userAgent).digest('hex') : null;
    const row = await this.dbs.withTx(null, (trx) =>
      trx
        .insertInto('sessions')
        .values({ user_id: userId, expires_at: expiresAt, user_agent_hash: uaHash })
        .returning('id')
        .executeTakeFirstOrThrow(),
    );
    const token = await new SignJWT({ sid: row.id })
      .setProtectedHeader({ alg: 'HS256' })
      .setSubject(userId)
      .setIssuedAt()
      .setExpirationTime(Math.floor(expiresAt.getTime() / 1000))
      .sign(this.key);
    return { sessionId: row.id, token, expiresAt };
  }

  /** The user behind a session cookie value, or null when it is invalid, expired or revoked. */
  async verify(token: string): Promise<RequestUser | null> {
    let userId: string;
    let sessionId: string;
    try {
      const { payload } = await jwtVerify(token, this.key, { algorithms: ['HS256'] });
      if (typeof payload.sub !== 'string' || typeof payload.sid !== 'string') return null;
      userId = payload.sub;
      sessionId = payload.sid;
    } catch {
      return null;
    }
    if (!(await this.isLive(sessionId, userId))) return null;
    return { userId, sessionId };
  }

  async revoke(sessionId: string): Promise<void> {
    await this.dbs.withTx(null, (trx) =>
      trx
        .updateTable('sessions')
        .set({ revoked_at: sql<Date>`now()` })
        .where('id', '=', sessionId)
        .where('revoked_at', 'is', null)
        .execute(),
    );
    try {
      await this.redis.del(sessionCacheKey(sessionId));
    } catch {
      this.logger.warn('session cache eviction failed; entry expires within 60 s');
    }
  }

  private async isLive(sessionId: string, userId: string): Promise<boolean> {
    const cacheKey = sessionCacheKey(sessionId);
    try {
      if ((await this.redis.get(cacheKey)) === userId) return true;
    } catch {
      // Redis is an optimization here; fall through to the database.
    }
    const row = await this.dbs.withTx(null, (trx) =>
      trx
        .selectFrom('sessions')
        .select('id')
        .where('id', '=', sessionId)
        .where('user_id', '=', userId)
        .where('revoked_at', 'is', null)
        .where('expires_at', '>', sql<Date>`now()`)
        .executeTakeFirst(),
    );
    if (!row) return false;
    try {
      await this.redis.set(cacheKey, userId, 'EX', SESSION_CACHE_SECONDS);
    } catch {
      // not cached; the next request asks the database again
    }
    return true;
  }
}
