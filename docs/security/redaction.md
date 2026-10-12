# Redaction

Second line of defense after "never log it". Patterns are defined per language and kept in
agreement through the shared corpus `packages/contracts/redaction-corpus.json`
(`positives[] {name, input, expected_redacted, pattern}` and `negatives[]`).

- TypeScript: `apps/api/src/telemetry/redact.ts` (`redactString`, `redactObject`, `redactAttributes`).
- Replacement format: `«redacted:<pattern>»`; redaction is idempotent.
- Sensitive keys (`authorization`, `cookie`, `password`, `token`, `secret`, `private_key`) are redacted regardless of value.
- Any single field over 4 KiB is truncated with `…[truncated]`.
- Object redaction is depth-limited (6) and circular safe.

Adding a pattern: add it in every implementation and add positive and negative corpus cases.
