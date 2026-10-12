import {
  BadRequestException,
  Controller,
  Inject,
  Logger,
  NotFoundException,
  Post,
  Req,
  Res,
  ServiceUnavailableException,
} from '@nestjs/common';
import { createHash } from 'node:crypto';
import type { Request, Response } from 'express';
import { Public } from '../auth/public.decorator';
import { incCounter } from '../common/metrics';
import { isIgnored, type ProviderKind } from '../providers/ports';
import { ProviderRegistry } from '../providers/provider.registry';
import {
  DELIVERY_STORE,
  PROVIDER_EVENT_SINK,
  type DeliveryRecord,
  type DeliveryStore,
  type ProviderEventSink,
} from './webhook.ports';
import type { WebhookAck } from './github-webhook.controller';

type NonGithub = Exclude<ProviderKind, 'github'>;

/**
 * `POST /api/v1/webhooks/gitlab` and `/bitbucket` (MP-001, MP-002). The same pipeline as the
 * GitHub endpoint, driven entirely through the provider ports: the registered provider
 * verifies the delivery (secret token or HMAC, constant time), the delivery is recorded for
 * idempotency, the provider normalizes it to a neutral event, and the event is handed to the
 * orchestrator while the answer stays a 202. Nothing from the payload is logged.
 */
@Public()
@Controller('webhooks')
export class ProviderWebhookController {
  private readonly logger = new Logger(ProviderWebhookController.name);

  constructor(
    private readonly registry: ProviderRegistry,
    @Inject(DELIVERY_STORE) private readonly deliveries: DeliveryStore,
    @Inject(PROVIDER_EVENT_SINK) private readonly sink: ProviderEventSink,
  ) {}

  @Post('gitlab')
  gitlab(@Req() req: Request, @Res() res: Response): Promise<void> {
    return this.receive('gitlab', req, res);
  }

  @Post('bitbucket')
  bitbucket(@Req() req: Request, @Res() res: Response): Promise<void> {
    return this.receive('bitbucket', req, res);
  }

  private async receive(kind: NonGithub, req: Request, res: Response): Promise<void> {
    // A provider that is not enabled is not registered: the route does not exist for it.
    if (!this.registry.has(kind)) throw new NotFoundException();
    const provider = this.registry.repository(kind);

    // The raw body is captured by the route-specific parser (app.setup.ts); an HMAC needs bytes.
    const rawBody = Buffer.isBuffer(req.body) ? req.body : Buffer.alloc(0);
    const verdict = provider.verifyWebhook(req.headers, rawBody);
    if (!verdict.valid) {
      incCounter('webhook_signature_failures_total', { reason: verdict.reason, provider: kind });
      this.logger.warn(`webhook rejected provider=${kind} reason=${verdict.reason}`);
      res.status(401).end();
      return;
    }

    let payload: unknown;
    try {
      payload = JSON.parse(rawBody.toString('utf8'));
    } catch {
      throw new BadRequestException('body is not valid JSON');
    }
    const { deliveryId, eventName } = verdict;
    const installationId = numericInstallationId(kind, payload);
    const delivery: DeliveryRecord = {
      provider: kind,
      deliveryId,
      eventName,
      ...(installationId ? { installationId } : {}),
      payloadSha256: createHash('sha256').update(rawBody).digest('hex'),
    };

    let ack: WebhookAck;
    try {
      const outcome = await this.deliveries.process(delivery, async () => {
        const normalized = await provider.normalizeEvent(req.headers, payload);
        // Neither provider has installation lifecycle events: only review events pass.
        if (isIgnored(normalized) || normalized.type === 'installation') {
          const reason = isIgnored(normalized) ? normalized.reason : 'unsupported_event';
          return {
            status: 'ignored' as const,
            ack: { delivery_id: deliveryId, accepted: false, reason },
          };
        }
        return {
          status: 'processed' as const,
          ack: { delivery_id: deliveryId, accepted: true },
          afterCommit: () => {
            // Detached on purpose: acknowledgement must not wait for orchestration.
            void this.sink.dispatch(normalized).catch((err: unknown) => {
              incCounter('webhook_dispatch_failures_total', { event: eventName, provider: kind });
              this.logger.error(`event dispatch failed delivery=${deliveryId} (${errorName(err)})`);
            });
          },
        };
      });
      ack = outcome.duplicate
        ? { delivery_id: deliveryId, accepted: false, reason: 'duplicate' }
        : outcome.ack;
    } catch (err) {
      // A failed store or handler must make the provider redeliver, never silently drop.
      this.logger.error(`delivery handling failed delivery=${deliveryId} (${errorName(err)})`);
      throw new ServiceUnavailableException();
    }

    incCounter('provider_webhooks_received_total', { provider: kind, accepted: ack.accepted });
    res.status(202).json(ack);
  }
}

/** The numeric id GitLab's payload names (`project.id`); Bitbucket has none. */
function numericInstallationId(kind: NonGithub, payload: unknown): string | undefined {
  if (kind !== 'gitlab') return undefined;
  const id = ((payload as { project?: { id?: unknown } } | null)?.project ?? {}).id;
  return typeof id === 'number' && Number.isInteger(id) && id > 0 ? String(id) : undefined;
}

function errorName(err: unknown): string {
  return err instanceof Error ? err.name : 'error';
}
