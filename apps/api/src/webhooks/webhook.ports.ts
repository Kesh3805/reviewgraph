import type { NormalizeResult, ProviderEvent } from '../providers/ports';

/** What the webhook endpoint knows about a verified delivery. */
export interface DeliveryRecord {
  deliveryId: string;
  eventName: string;
  action?: string;
  installationId?: string;
}

export type DeliveryOutcome = 'new' | 'duplicate';

/**
 * Idempotency store (GH-003: Redis SETNX plus the `webhook_deliveries` table). A failure here
 * must surface as an error so the endpoint answers 503 and GitHub retries.
 */
export interface DeliveryStore {
  record(delivery: DeliveryRecord): Promise<DeliveryOutcome>;
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
