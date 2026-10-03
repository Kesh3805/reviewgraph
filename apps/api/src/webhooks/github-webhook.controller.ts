import {
  BadRequestException,
  Controller,
  Headers,
  HttpStatus,
  Inject,
  Logger,
  NotFoundException,
  Post,
  Req,
  Res,
  ServiceUnavailableException,
} from '@nestjs/common';
import { SpanStatusCode, trace } from '@opentelemetry/api';
import { createHash } from 'node:crypto';
import type { Request, Response } from 'express';
import { Public } from '../auth/public.decorator';
import { incCounter } from '../common/metrics';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { isIgnored } from '../providers/ports';
import { TRACER_NAME } from '../telemetry/tracer.service';
import { verifyWebhookSignature } from './signature';
import {
  COMMAND_ACKNOWLEDGER,
  DELIVERY_STORE,
  EVENT_NORMALIZER,
  INSTALLATION_LIFECYCLE,
  PROVIDER_EVENT_SINK,
  type CommandAcknowledger,
  type DeliveryContext,
  type DeliveryRecord,
  type DeliveryStore,
  type DeliveryWorkResult,
  type EventNormalizer,
  type InstallationLifecycle,
  type ProviderEventSink,
} from './webhook.ports';

/** Events the GitHub App subscribes to (the permission manifest, GH-010). */
export const SUPPORTED_GITHUB_EVENTS: ReadonlySet<string> = new Set([
  'pull_request',
  'issue_comment',
  'installation',
  'installation_repositories',
]);

export interface WebhookAck {
  delivery_id: string;
  accepted: boolean;
  reason?: string;
}

/**
 * `POST /api/v1/webhooks/github` (GH-002): verify the HMAC over the raw body, record the
 * delivery, normalize, hand off, and answer 202 without waiting for review work.
 * Nothing from the payload is logged: only event, action, delivery id and installation id.
 */
@Public()
@Controller('webhooks')
export class GithubWebhookController {
  private readonly logger = new Logger(GithubWebhookController.name);

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(DELIVERY_STORE) private readonly deliveries: DeliveryStore,
    @Inject(EVENT_NORMALIZER) private readonly normalizer: EventNormalizer,
    @Inject(PROVIDER_EVENT_SINK) private readonly sink: ProviderEventSink,
    @Inject(COMMAND_ACKNOWLEDGER) private readonly acknowledger: CommandAcknowledger,
    @Inject(INSTALLATION_LIFECYCLE) private readonly installations: InstallationLifecycle,
  ) {}

  @Post('github')
  async receive(
    @Req() req: Request,
    @Res() res: Response,
    @Headers('x-hub-signature-256') signature: string | undefined,
    @Headers('x-github-event') eventName: string | undefined,
    @Headers('x-github-delivery') deliveryId: string | undefined,
  ): Promise<void> {
    if (!this.config.GITHUB_ENABLED) throw new NotFoundException();

    // The raw body is captured by the route-specific parser (app.setup.ts); HMAC needs the bytes.
    const rawBody = Buffer.isBuffer(req.body) ? req.body : Buffer.alloc(0);
    const secrets = [
      this.config.GITHUB_WEBHOOK_SECRET,
      this.config.GITHUB_WEBHOOK_SECRET_PREVIOUS,
    ].filter((s): s is string => Boolean(s));
    const verdict = verifyWebhookSignature(secrets, rawBody, signature);
    if (verdict !== 'valid') {
      incCounter('webhook_signature_failures_total', { reason: verdict });
      // Empty body, no payload in the log line.
      this.logger.warn(
        `webhook signature rejected reason=${verdict} delivery=${deliveryId ?? '-'}`,
      );
      res.status(HttpStatus.UNAUTHORIZED).end();
      return;
    }
    if (!eventName || !deliveryId) {
      throw new BadRequestException('missing X-GitHub-Event or X-GitHub-Delivery');
    }

    let payload: unknown;
    try {
      payload = JSON.parse(rawBody.toString('utf8'));
    } catch {
      throw new BadRequestException('body is not valid JSON');
    }
    const action = stringField(payload, 'action');
    const installationId = installationIdOf(payload);

    const ack = await trace
      .getTracer(TRACER_NAME)
      .startActiveSpan(
        'webhook_received',
        { attributes: { event: eventName, action: action ?? '', delivery_id: deliveryId } },
        async (span) => {
          try {
            return await this.process(
              { deliveryId, eventName, action, installationId, payloadSha256: sha256(rawBody) },
              payload,
            );
          } catch (err) {
            span.setStatus({ code: SpanStatusCode.ERROR });
            throw err;
          } finally {
            span.end();
          }
        },
      );

    incCounter('webhooks_received_total', { event: eventName, accepted: ack.accepted });
    this.logger.log(
      `webhook event=${eventName} action=${action ?? '-'} delivery=${deliveryId} ` +
        `installation=${installationId ?? '-'} accepted=${ack.accepted} reason=${ack.reason ?? '-'}`,
    );
    res.status(HttpStatus.ACCEPTED).json(ack);
  }

  private async process(delivery: DeliveryRecord, payload: unknown): Promise<WebhookAck> {
    const { deliveryId, eventName } = delivery;
    if (eventName === 'ping') return { delivery_id: deliveryId, accepted: false, reason: 'ping' };
    // Unsupported events are acknowledged with 202 so GitHub does not retry them.
    if (!SUPPORTED_GITHUB_EVENTS.has(eventName)) {
      return { delivery_id: deliveryId, accepted: false, reason: 'unsupported_event' };
    }

    let outcome;
    try {
      // Records the delivery and runs the handling in one transaction (GH-003): a duplicate is
      // never handled twice, and a failure rolls the record back so GitHub's retry runs fresh.
      outcome = await this.deliveries.process(delivery, (ctx) =>
        this.handle(delivery, ctx, payload),
      );
    } catch (err) {
      // A failed store or handler must make GitHub redeliver, never silently drop the event.
      this.logger.error(`delivery handling failed delivery=${deliveryId} (${errorName(err)})`);
      throw new ServiceUnavailableException();
    }
    if (outcome.duplicate) {
      return { delivery_id: deliveryId, accepted: false, reason: 'duplicate' };
    }
    return outcome.ack;
  }

  private async handle(
    delivery: DeliveryRecord,
    ctx: DeliveryContext,
    payload: unknown,
  ): Promise<DeliveryWorkResult<WebhookAck>> {
    const { deliveryId, eventName } = delivery;
    const normalized = await this.normalizer.normalize(eventName, payload, deliveryId);
    if (isIgnored(normalized)) {
      return {
        status: 'ignored',
        ack: { delivery_id: deliveryId, accepted: false, reason: normalized.reason },
      };
    }
    if (normalized.type === 'installation') {
      // Lifecycle events change tenancy and access; they run in the delivery transaction and
      // never reach the review orchestrator.
      const outcome = await this.installations.apply(ctx, normalized);
      if (!outcome.applied) {
        return {
          status: 'ignored',
          ack: { delivery_id: deliveryId, accepted: false, reason: 'unknown_installation' },
        };
      }
      return {
        status: 'processed',
        ack: { delivery_id: deliveryId, accepted: true },
        organizationId: outcome.organizationId,
        afterCommit: outcome.afterCommit,
      };
    }
    return {
      status: 'processed',
      ack: { delivery_id: deliveryId, accepted: true },
      afterCommit: () => {
        // Detached on purpose: acknowledgement must not wait for orchestration.
        void this.sink.dispatch(normalized).catch((err: unknown) => {
          incCounter('webhook_dispatch_failures_total', { event: eventName });
          this.logger.error(`event dispatch failed delivery=${deliveryId} (${errorName(err)})`);
        });
        if (normalized.type === 'review_command') {
          // Best effort: a failed reaction must not affect the review.
          void this.acknowledger.acknowledge(normalized).catch((err: unknown) => {
            this.logger.warn(
              `command acknowledgement failed delivery=${deliveryId} (${errorName(err)})`,
            );
          });
        }
      },
    };
  }
}

function stringField(payload: unknown, key: string): string | undefined {
  const value = (payload as Record<string, unknown> | null)?.[key];
  return typeof value === 'string' ? value : undefined;
}

function installationIdOf(payload: unknown): string | undefined {
  const id = (
    (payload as Record<string, unknown> | null)?.installation as { id?: unknown } | undefined
  )?.id;
  return typeof id === 'number' || typeof id === 'string' ? String(id) : undefined;
}

function sha256(body: Buffer): string {
  return createHash('sha256').update(body).digest('hex');
}

function errorName(err: unknown): string {
  return err instanceof Error ? err.name : 'error';
}
