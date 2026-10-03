import { Injectable } from '@nestjs/common';
import type { Tx } from '../db/tx';

/**
 * Reads the computed repository profile (PROF-001 JSON, `repository_profiles`). The profile
 * pipeline and its table are a later task: until then there is never a profile, so
 * `GET /repositories/:id/profile` answers 404.
 */
export interface ProfileReader {
  /** The profile JSON and when it was computed, or null before the first index. */
  read(trx: Tx, repositoryId: string): Promise<{ profile: unknown; computedAt: Date } | null>;
}
export const PROFILE_READER = Symbol('PROFILE_READER');

@Injectable()
export class NullProfileReader implements ProfileReader {
  read(): Promise<null> {
    return Promise.resolve(null);
  }
}
