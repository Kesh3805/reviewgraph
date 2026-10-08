# Webhook replay protection

GitHub signs the webhook body (HMAC-SHA256, GH-002) but the signature carries no timestamp, so a
captured request stays valid forever. The API bounds the replay surface in four layers, in this
order, all after a valid signature (an unauthenticated caller can never reach the guard, so it
cannot be probed):

1. **Per-installation intake limit.** A Redis fixed window counts deliveries per installation per
   minute. Above `WEBHOOK_INSTALLATION_RATE_LIMIT` (default 120, `0` disables) the endpoint answers
   `429` with `Retry-After` and an empty body; GitHub retries later.
2. **Delivery id reuse.** GitHub redelivers the *same* body under the same `X-GitHub-Delivery`. A
   recorded delivery id whose stored body hash (`webhook_deliveries.payload_sha256`) differs is a
   replay with a substituted body: it is acknowledged `202 {accepted:false, reason:"delivery_id_reuse"}`,
   never processed, counted in `webhook_replays_rejected_total{reason="delivery_id_reuse"}` and
   written to the audit log (`webhook.delivery_id_reuse`, outcome `denied`) when the installation's
   organization is known.
3. **Freshness.** The newest timestamp the signed body carries (`pull_request.updated_at`,
   `head_commit.timestamp`, `check_suite.updated_at`) older than `WEBHOOK_MAX_EVENT_AGE_SECONDS`
   (default 900) makes the delivery `202 {accepted:false, reason:"stale_event"}`. It is still
   recorded, so a later replay of it is a plain duplicate. Events without a timestamp (installation,
   comments) are not aged. The real processing guard stays the head check of supersession
   (SUP-001): an event for a head that is no longer current does nothing.
4. **Idempotency (GH-003).** A same-body duplicate is answered `reason:"duplicate"` by the Redis
   `SET NX` fast path or, after its 72 h TTL, by the `webhook_deliveries` row.

## Retention and tuning

- `webhook_deliveries` rows (delivery id, body hash, status, timestamps; never the body) must be
  kept at least `WEBHOOK_REPLAY_RETENTION_DAYS` (default 30) so a replay after the Redis TTL is
  still detected. The SEC-007 purge reads this setting; `webhook_deliveries_received_idx` serves it.
- A failed delivery rolls its row back, so GitHub's (or an operator's) redelivery of it is
  processed normally.
- Raise `WEBHOOK_MAX_EVENT_AGE_SECONDS` if deliveries are queued for long on GitHub's side (an
  outage); the polling reconciler (GH-012) catches anything ignored as stale.
- The guard fails open on its own errors: Redis down disables the intake limit, and a failed hash
  lookup skips the reuse check; Postgres being down still answers 503 so GitHub retries.
- IP allow-listing of GitHub's hook ranges is an optional infrastructure control and is not done
  by the API.

## Metrics

`webhook_replays_rejected_total{reason=rate_limited|delivery_id_reuse}`,
`webhook_stale_events_total`, alongside `webhook_signature_failures_total` (GH-002) and
`webhook_duplicates_total` (GH-003).
