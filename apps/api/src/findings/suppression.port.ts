import { Injectable, ServiceUnavailableException } from '@nestjs/common';
import type { Tx } from '../db/tx';
import type { SuppressionKind } from './dto/feedback.dto';

export interface NewSuppression {
  organizationId: string;
  repositoryId: string;
  kind: SuppressionKind;
  /** The fingerprint, symbol id or path the suppression matches. */
  value: string;
  reason: string;
  createdBy: string;
}

/**
 * Creates API-sourced suppressions (POL-006 owns the `suppressions` table and the matching
 * verification stage). It runs in the feedback transaction, so the feedback, the suppression and
 * their audit rows commit together.
 */
export interface SuppressionWriter {
  /** Returns the suppression id. */
  create(trx: Tx, suppression: NewSuppression): Promise<string>;
}
export const SUPPRESSION_WRITER = Symbol('SUPPRESSION_WRITER');

/** Default until POL-006 adds the `suppressions` table: creating one answers 503. */
@Injectable()
export class UnwiredSuppressionWriter implements SuppressionWriter {
  create(): Promise<string> {
    return Promise.reject(
      new ServiceUnavailableException('creating suppressions is not available yet'),
    );
  }
}
