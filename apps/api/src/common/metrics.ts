import { metrics, type Attributes, type Counter } from '@opentelemetry/api';

const METER_NAME = 'reviewgraph-api';
const counters = new Map<string, Counter>();
const totals = new Map<string, number>();

function keyOf(name: string, attrs: Attributes): string {
  const parts = Object.entries(attrs)
    .map(([k, v]) => `${k}=${String(v)}`)
    .sort();
  return `${name}{${parts.join(',')}}`;
}

/**
 * Increments a monotonic counter. Instruments are created lazily so the global meter provider
 * registered by the SDK (OBS-002) is used; without an SDK this is a no-op export-wise. A local
 * tally is kept so tests can assert on counts without a metrics pipeline.
 */
export function incCounter(name: string, attrs: Attributes = {}, value = 1): void {
  let counter = counters.get(name);
  if (!counter) {
    counter = metrics.getMeter(METER_NAME).createCounter(name);
    counters.set(name, counter);
  }
  counter.add(value, attrs);
  const key = keyOf(name, attrs);
  totals.set(key, (totals.get(key) ?? 0) + value);
}

/** Local tally for a counter + attribute set (tests and diagnostics). */
export function counterTotal(name: string, attrs: Attributes = {}): number {
  return totals.get(keyOf(name, attrs)) ?? 0;
}

/** Sum of a counter across every attribute set. */
export function counterTotalAll(name: string): number {
  let sum = 0;
  for (const [key, value] of totals) if (key.startsWith(`${name}{`)) sum += value;
  return sum;
}

export function resetCounterTotals(): void {
  totals.clear();
}

const gauges = new Map<string, Map<string, { value: number; attrs: Attributes }>>();

/** Sets an observable gauge value for an attribute set. */
export function setGauge(name: string, value: number, attrs: Attributes = {}): void {
  let series = gauges.get(name);
  if (!series) {
    series = new Map();
    gauges.set(name, series);
    const live = series;
    metrics
      .getMeter(METER_NAME)
      .createObservableGauge(name)
      .addCallback((result) => {
        for (const { value: v, attrs: a } of live.values()) result.observe(v, a);
      });
  }
  series.set(keyOf(name, attrs), { value, attrs });
}

export function gaugeValue(name: string, attrs: Attributes = {}): number | undefined {
  return gauges.get(name)?.get(keyOf(name, attrs))?.value;
}
