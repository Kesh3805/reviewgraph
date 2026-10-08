/**
 * The audit event catalog (SEC-008). Actions are dotted `<target>.<verb>` names; every action
 * the API writes is listed here, so the read API and dashboards can rely on a closed set.
 */
export const AUDIT_ACTIONS = [
  'repository.enabled',
  'repository.settings.updated',
  'review.manual_triggered',
  'review.cancelled',
  'finding.feedback.created',
  'finding.feedback.updated',
  'suppression.created',
  'source.excerpt.read',
  'review.published',
  'membership.changed',
  'auth.login',
  'auth.logout',
  'credentials.issued',
  'service_token.replay_rejected',
  'access.denied',
] as const;

export type AuditAction = (typeof AUDIT_ACTIONS)[number];

export type AuditActorType = 'user' | 'service' | 'system';
export type AuditOutcome = 'success' | 'denied' | 'failure';

export interface AuditEvent {
  organizationId: string;
  repositoryId?: string;
  actor: { type: AuditActorType; id?: string };
  /**
   * Dotted action name, for example `repository.settings.updated`. New actions are added to
   * `AUDIT_ACTIONS` (a unit test checks that every action written by the API is cataloged).
   */
  action: string;
  targetType: string;
  targetId?: string;
  outcome?: AuditOutcome;
  /** Ids, enum values and before/after of non-secret fields only; redacted again on write. */
  metadata?: Record<string, unknown>;
  requestId?: string;
  /**
   * Makes the event idempotent per organization (for example `publish:{run}:{head}`): a retried
   * job writes it once.
   */
  dedupeKey?: string;
}
