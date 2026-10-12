//! The central metric registry (OBS-005).
//!
//! Every instrument of the catalog ([`instruments::CATALOG`]) is created once from a meter and
//! looked up by name. Recording is lock-free for counters and histograms; a bad label name is
//! dropped and a bad label value becomes `other` instead of panicking. Gauges also keep their
//! last value so tests and background refresh tasks can read it back.
//!
//! Metrics are exported over OTLP only; there is no Prometheus endpoint in the MVP.

pub mod buckets;
pub mod catalog;
pub mod instruments;
pub mod labels;
pub mod rates;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};
use opentelemetry::KeyValue;

pub use instruments::{Kind, Spec, CATALOG};

/// Meter name of the registry.
pub const METER_NAME: &str = "reviewgraph";

enum Instrument {
    Counter(Counter<u64>),
    Histogram(Histogram<f64>),
    Gauge(Gauge<f64>),
}

/// All instruments of the catalog, created once from one meter.
pub struct Registry {
    instruments: HashMap<&'static str, (&'static Spec, Instrument)>,
    gauge_values: Mutex<BTreeMap<String, f64>>,
    rejected: AtomicU64,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("instruments", &self.instruments.len())
            .field("rejected", &self.rejected.load(Ordering::Relaxed))
            .finish()
    }
}

impl Registry {
    /// Creates every catalog instrument on `meter`. Duplicate names in the catalog are a
    /// programming error caught by the catalog tests, not at runtime.
    pub fn new(meter: &Meter) -> Self {
        let mut instruments = HashMap::new();
        for spec in CATALOG {
            let instrument = match spec.kind {
                Kind::Counter => Instrument::Counter(
                    meter
                        .u64_counter(spec.name)
                        .with_unit(spec.unit)
                        .with_description(spec.description)
                        .build(),
                ),
                Kind::Histogram => Instrument::Histogram(
                    meter
                        .f64_histogram(spec.name)
                        .with_unit(spec.unit)
                        .with_description(spec.description)
                        .with_boundaries(buckets::DURATION_SECONDS.to_vec())
                        .build(),
                ),
                Kind::Gauge => Instrument::Gauge(
                    meter
                        .f64_gauge(spec.name)
                        .with_unit(spec.unit)
                        .with_description(spec.description)
                        .build(),
                ),
            };
            instruments.insert(spec.name, (spec, instrument));
        }
        Self {
            instruments,
            gauge_values: Mutex::new(BTreeMap::new()),
            rejected: AtomicU64::new(0),
        }
    }

    /// Names of every registered instrument.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<_> = self.instruments.keys().copied().collect();
        names.sort_unstable();
        names
    }

    /// The type of the instrument registered under `name`.
    pub fn kind_of(&self, name: &str) -> Option<Kind> {
        self.instruments.get(name).map(|(spec, _)| spec.kind)
    }

    /// Calls dropped because the metric name is unknown or the instrument has another type.
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    fn attributes(spec: &Spec, labels: &[(&str, &str)]) -> Vec<KeyValue> {
        labels
            .iter()
            .filter(|(name, _)| spec.labels.iter().any(|l| l == name))
            .map(|(name, value)| {
                KeyValue::new(name.to_string(), labels::bound(name, value).into_owned())
            })
            .collect()
    }

    /// Adds `value` to a counter.
    pub fn add(&self, name: &str, value: u64, labels: &[(&str, &str)]) {
        match self.instruments.get(name) {
            Some((spec, Instrument::Counter(c))) => c.add(value, &Self::attributes(spec, labels)),
            _ => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Records one histogram observation (seconds for durations).
    pub fn observe(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        match self.instruments.get(name) {
            Some((spec, Instrument::Histogram(h))) => {
                h.record(value, &Self::attributes(spec, labels))
            }
            _ => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Sets a gauge and remembers the value for [`Registry::gauge_value`].
    pub fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        match self.instruments.get(name) {
            Some((spec, Instrument::Gauge(g))) => {
                let attrs = Self::attributes(spec, labels);
                g.record(value, &attrs);
                if let Ok(mut cache) = self.gauge_values.lock() {
                    cache.insert(series_key(name, &attrs), value);
                }
            }
            _ => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// The last value set for a gauge series (labels are bounded the same way as on write).
    pub fn gauge_value(&self, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
        let (spec, _) = self.instruments.get(name)?;
        let attrs = Self::attributes(spec, labels);
        self.gauge_values
            .lock()
            .ok()?
            .get(&series_key(name, &attrs))
            .copied()
    }
}

fn series_key(name: &str, attrs: &[KeyValue]) -> String {
    let mut parts: Vec<String> = attrs
        .iter()
        .map(|kv| format!("{}={}", kv.key.as_str(), kv.value))
        .collect();
    parts.sort();
    format!("{name}{{{}}}", parts.join(","))
}

static GLOBAL: OnceLock<Registry> = OnceLock::new();

/// The process-wide registry on the global meter. Created on first use; `telemetry::init`
/// forces it right after installing the meter provider, so call `init` first.
pub fn global() -> &'static Registry {
    GLOBAL.get_or_init(|| Registry::new(&opentelemetry::global::meter(METER_NAME)))
}
