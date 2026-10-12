import type { Attributes } from '@opentelemetry/api';
import { incCounter, recordHistogram, setGauge } from '../common/metrics';

/**
 * TypeScript twin of the Rust metric registry (OBS-005). It defines the metrics TS owns and
 * must agree with `docs/operations/metrics-catalog.md`, which is generated from
 * `engine/crates/telemetry/src/metrics/instruments.rs`; `test/telemetry/metrics.spec.ts` fails
 * on drift. Labels come from the allow-list; a value outside its domain becomes `other`.
 */
export type MetricKind = 'counter' | 'histogram' | 'gauge';

export interface MetricSpec {
  readonly kind: MetricKind;
  readonly unit: string;
  readonly labels: readonly string[];
}

export const LABEL_ALLOW_LIST = [
  'reviewer_type',
  'model',
  'provider',
  'task',
  'tier',
  'queue',
  'outcome',
  'stage',
  'language',
  'severity',
  'category',
  'state',
  'kind',
  'reason',
  'pattern',
  'op',
  'policy',
] as const;

/** Identifiers never label a metric (cardinality and tenant leakage). */
export const FORBIDDEN_LABELS = [
  'organization_id',
  'repository_id',
  'review_run_id',
  'pull_request_id',
  'commit_sha',
  'job_id',
  'request_id',
  'path',
  'file_path',
] as const;

/** Duration histogram bucket boundaries in seconds (shared with Rust). */
export const DURATION_BUCKETS_SECONDS = [
  0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60, 120, 300, 900,
] as const;

export const OTHER = 'other';

const counter = (labels: readonly string[]): MetricSpec => ({ kind: 'counter', unit: '1', labels });
const histogram = (labels: readonly string[]): MetricSpec => ({
  kind: 'histogram',
  unit: 's',
  labels,
});
const gauge = (labels: readonly string[], unit = '1'): MetricSpec => ({
  kind: 'gauge',
  unit,
  labels,
});

/** The metrics shared with Rust (names, kinds, units and labels must match the catalog). */
export const TS_METRICS = {
  queue_depth: gauge(['queue', 'state']),
  queue_wait_seconds: histogram(['queue']),
  worker_duration_seconds: histogram(['queue', 'outcome']),
  published_findings_total: counter(['reviewer_type', 'category', 'severity']),
  finding_acceptance_rate: gauge(['reviewer_type'], 'ratio'),
  finding_false_positive_rate: gauge(['reviewer_type'], 'ratio'),
  webhook_signature_failures_total: counter([]),
  tenancy_denied_total: counter([]),
  trace_context_missing_total: counter(['queue']),
  log_redactions_total: counter(['pattern']),
  github_token_mint_total: counter(['outcome']),
  clone_credentials_issued_total: counter([]),
  token_cache_decrypt_failures_total: counter([]),
  object_store_requests_total: counter(['op', 'outcome']),
  object_store_denied_total: counter([]),
} as const satisfies Record<string, MetricSpec>;

export type TsMetricName = keyof typeof TS_METRICS;

const CLOSED: Record<string, readonly string[]> = {
  reviewer_type: [
    'correctness',
    'security',
    'test',
    'architecture',
    'performance',
    'maintainability',
  ],
  queue: [
    'repository-index',
    'incremental-index',
    'pr-review',
    'review-publish',
    'history-ingest',
    'retention-sweep',
  ],
  state: ['queued', 'running', 'succeeded', 'failed', 'dead', 'cancelled'],
  severity: ['info', 'low', 'medium', 'high', 'critical'],
  outcome: [
    'ok',
    'error',
    'failed',
    'skipped',
    'superseded',
    'timeout',
    'denied',
    'success',
    'retry',
    'cache_hit',
    'cache_miss',
    'unavailable',
  ],
};

const OPEN_TOKEN = /^[a-z0-9._:-]{1,64}$/;

/** Maps `value` into the domain of label `name`; anything else becomes `other`. */
export function boundLabel(name: string, value: string): string {
  const closed = CLOSED[name];
  if (closed) return closed.includes(value) ? value : OTHER;
  return OPEN_TOKEN.test(value) ? value : OTHER;
}

function attributesFor(name: TsMetricName, labels: Record<string, string>): Attributes {
  const allowed: readonly string[] = TS_METRICS[name].labels;
  const out: Attributes = {};
  for (const [k, v] of Object.entries(labels)) {
    if (allowed.includes(k)) out[k] = boundLabel(k, v);
  }
  return out;
}

function assertKind(name: TsMetricName, kind: MetricKind): void {
  if (TS_METRICS[name].kind !== kind) throw new Error(`${name} is a ${TS_METRICS[name].kind}`);
}

export function addMetric(
  name: TsMetricName,
  labels: Record<string, string> = {},
  value = 1,
): void {
  assertKind(name, 'counter');
  incCounter(name, attributesFor(name, labels), value);
}

export function observeMetric(
  name: TsMetricName,
  seconds: number,
  labels: Record<string, string> = {},
): void {
  assertKind(name, 'histogram');
  recordHistogram(name, seconds, attributesFor(name, labels));
}

export function setMetricGauge(
  name: TsMetricName,
  value: number,
  labels: Record<string, string> = {},
): void {
  assertKind(name, 'gauge');
  setGauge(name, value, attributesFor(name, labels));
}
