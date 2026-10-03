//! Price table and the cost formula (GW-008).
//!
//! 1 USD per million tokens is exactly 1 micro-USD per token, so
//! `cost_usd_micros = round_half_up(input_uncached*p_in + cache_write*p_cw + cache_read*p_cr + output*p_out)`
//! with `p_*` in USD/MTok. Reasoning tokens are already part of `output`.

use std::str::FromStr;

use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Deserialize;

use crate::budget::WorstCasePricer;
use crate::error::Error;
use crate::types::{ProviderId, Usage};

/// The bundled table.
pub const DEFAULT_PRICES_YAML: &str = include_str!("../../config/prices.yaml");

/// Entries older than this fail the staleness check.
pub const MAX_PRICE_AGE_DAYS: i64 = 120;

#[derive(Debug, Deserialize)]
struct PricesFile {
    version: u32,
    prices: Vec<RawEntry>,
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    provider: String,
    model: String,
    usd_per_mtok: RawRates,
    as_of: String,
    source: String,
}

#[derive(Debug, Deserialize)]
struct RawRates {
    input: String,
    output: String,
    cache_write: String,
    cache_read: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriceEntry {
    pub provider: String,
    pub model: String,
    pub input: Decimal,
    pub output: Decimal,
    pub cache_write: Decimal,
    pub cache_read: Decimal,
    pub as_of: NaiveDate,
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub struct PriceTable {
    entries: Vec<PriceEntry>,
}

fn dec(field: &str, v: &str) -> Result<Decimal, Error> {
    Decimal::from_str(v).map_err(|e| Error::Config(format!("price `{field}` = `{v}`: {e}")))
}

impl PriceTable {
    pub fn from_yaml(yaml: &str) -> Result<Self, Error> {
        let file: PricesFile = serde_yaml::from_str(yaml)
            .map_err(|e| Error::Config(format!("invalid prices yaml: {e}")))?;
        if file.version != 1 {
            return Err(Error::Config(format!(
                "unsupported prices version {}",
                file.version
            )));
        }
        let mut entries = Vec::new();
        for e in file.prices {
            let as_of = NaiveDate::from_str(&e.as_of)
                .map_err(|err| Error::Config(format!("price as_of `{}`: {err}", e.as_of)))?;
            if e.source.trim().is_empty() {
                return Err(Error::Config(format!(
                    "price for {} has no source",
                    e.model
                )));
            }
            entries.push(PriceEntry {
                provider: e.provider,
                model: e.model,
                input: dec("input", &e.usd_per_mtok.input)?,
                output: dec("output", &e.usd_per_mtok.output)?,
                cache_write: dec("cache_write", &e.usd_per_mtok.cache_write)?,
                cache_read: dec("cache_read", &e.usd_per_mtok.cache_read)?,
                as_of,
                source: e.source,
            });
        }
        Ok(Self { entries })
    }

    pub fn default_table() -> Result<Self, Error> {
        Self::from_yaml(DEFAULT_PRICES_YAML)
    }

    pub fn entries(&self) -> &[PriceEntry] {
        &self.entries
    }

    pub fn get(&self, provider: &str, model: &str) -> Option<&PriceEntry> {
        self.entries
            .iter()
            .find(|e| e.provider == provider && e.model == model)
    }

    /// Entries whose `as_of` is more than `max_age_days` before `today`.
    pub fn stale(&self, today: NaiveDate, max_age_days: i64) -> Vec<&PriceEntry> {
        self.entries
            .iter()
            .filter(|e| (today - e.as_of).num_days() > max_age_days)
            .collect()
    }

    /// Cost of `usage` on a priced model; `None` for an unpriced one.
    pub fn cost_micros(&self, provider: &str, model: &str, usage: &Usage) -> Option<u64> {
        let p = self.get(provider, model)?;
        let total = Decimal::from(usage.input_uncached) * p.input
            + Decimal::from(usage.cache_write) * p.cache_write
            + Decimal::from(usage.cache_read) * p.cache_read
            + Decimal::from(usage.output) * p.output;
        total
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
            .to_u64()
    }

    /// The date of the price used for `model`, for the ledger.
    pub fn as_of(&self, provider: &str, model: &str) -> Option<NaiveDate> {
        self.get(provider, model).map(|e| e.as_of)
    }
}

impl WorstCasePricer for PriceTable {
    fn worst_case_micros(
        &self,
        provider: &ProviderId,
        model: &str,
        input: u32,
        output: u32,
    ) -> Option<u64> {
        // An unpriced model is bounded by the most expensive priced model of its provider.
        let entry = self.get(provider.as_str(), model).or_else(|| {
            self.entries
                .iter()
                .filter(|e| e.provider == provider.as_str())
                .max_by_key(|e| e.output)
        })?;
        // Input may be billed at the cache-write rate, so use the larger of the two.
        let in_rate = entry.input.max(entry.cache_write);
        let total = Decimal::from(input) * in_rate + Decimal::from(output) * entry.output;
        total
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
            .to_u64()
    }
}
