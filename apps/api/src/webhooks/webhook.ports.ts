import type { Tx } from '../db/tx';
import type { NormalizeResult, ProviderEvent, ReviewCommandEvent } from '../providers/ports';

/** What the webhook endpoint knows about a verified delivery. */
export interface DeliveryRecord {
  deliveryId: string;
  eventName: string;
  action?: string;
  installationId?: string;
  /** SHA-256 of the raw body; the payload itself is never stored. */
  payloadSha256: string;
}

/** Handed to the work of a delivery: the transaction that holds its `webhook_deliveries` row. */
export interface DeliveryContext {
  trx: Tx;
}

export interface DeliveryWorkResult<A> {
  /** Stored as `webhook_deliveries.status`. */
  status: 'processed' | 'ignored';
  /** The webhook acknowledgement body. */
  ack: A;
  /** Set once the tenant of the delivery is known. */
  organizationId?: string;
  /** Runs only after the transaction committed (dispatch to the orchestrator, reactions). */
  afterCommit?: () => void;
}

export type DeliveryOutcome<A> = { duplicate: true } | { duplicate: false; ack: A };

/**
 * Idempotency store (GH-003): a Redis `SET NX` fast path in front of the durable
 * `webhook_deliveries` row. `process` records the delivery and runs `work` in ONE database
 * transaction: a duplicate never runs `work`; if `work` (or the commit) fails, the row rolls back
 * so GitHub's retry is processed fresh. A store failure must surface as an error so the endpoint
 * answers 503 and GitHub retries.
 */
export interface DeliveryStore {
  process<A>(
    delivery: DeliveryRecord,
    work: (ctx: DeliveryContext) => Promise<DeliveryWorkResult<A>>,
  ): Promise<DeliveryOutcome<A>>;
}
export const DELIVERY_STORE = Symbol('DELIVERY_STORE');

/** Turns a verified payload into a provider-neutral event (GH-004). */
export interface EventNormalizer {
  normalize(eventName: string, payload: unknown, deliveryId: string): Promise<NormalizeResult>;
}
export const EVENT_NORMALIZER = Symbol('EVENT_NORMALIZER');

/**
 * Hands a normalized event to the PR review orchestrator (SUP-001). The acknowledgement never
 * waits for it: the endpoint dispatches and returns 202.
 */
export interface ProviderEventSink {
  dispatch(event: ProviderEvent): Promise<void>;
}
export const PROVIDER_EVENT_SINK = Symbol('PROVIDER_EVENT_SINK');

/** Best-effort acknowledgement of an accepted /review command (an emoji reaction). */
export interface CommandAcknowledger {
  acknowledge(event: ReviewCommandEvent): Promise<void>;
}
export const COMMAND_ACKNOWLEDGER = Symbol('COMMAND_ACKNOWLEDGER');
