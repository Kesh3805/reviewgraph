//! Miner registry and the parallel mining driver (PROF-003).

use std::collections::BTreeSet;
use std::time::Instant;

use chrono::{DateTime, Utc};
use rayon::prelude::*;

use super::confidence::{confidence, consistency, is_enforceable};
use super::miner::{Conformance, ConventionMiner, GraphView, PartialProfile};
use super::scope::{compile_all, exception_reason, narrow_scope, Judged};
use crate::metrics;
use crate::model::{Convention, ConventionException, ConventionSource, PROFILE_VERSION};

/// Inputs shared by all miners.
#[derive(Debug, Clone, Default)]
pub struct MineOptions {
    /// Generated-file globs (violations there are exceptions).
    pub generated_globs: Vec<String>,
    /// `conventions.exceptions` from config.
    pub exception_globs: Vec<String>,
}

#[derive(Default)]
pub struct MinerRegistry {
    miners: Vec<Box<dyn ConventionMiner>>,
}

impl MinerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, miner: Box<dyn ConventionMiner>) {
        self.miners.push(miner);
    }

    /// `(id, version)` of every registered miner, sorted (input to the profile version).
    pub fn versions(&self) -> Vec<(&'static str, u32)> {
        let mut v: Vec<_> = self.miners.iter().map(|m| (m.id(), m.version())).collect();
        v.sort_unstable();
        v
    }

    /// Runs every miner in parallel; a miner producing no population yields no convention.
    /// Output order is by convention id, so the result is deterministic.
    pub fn mine_all(
        &self,
        g: &dyn GraphView,
        partial: &PartialProfile,
        opts: &MineOptions,
        now: DateTime<Utc>,
    ) -> Vec<Convention> {
        let mut out: Vec<Convention> = self
            .miners
            .par_iter()
            .filter_map(|m| mine_one(m.as_ref(), g, partial, opts, now))
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

/// Mines one convention. `None` when the population is empty or nothing is applicable.
pub fn mine_one(
    miner: &dyn ConventionMiner,
    g: &dyn GraphView,
    partial: &PartialProfile,
    opts: &MineOptions,
    now: DateTime<Utc>,
) -> Option<Convention> {
    let started = Instant::now();
    let generated = compile_all(&opts.generated_globs);
    let configured = compile_all(&opts.exception_globs);

    let mut judged: Vec<Judged> = Vec::new();
    let mut exceptions: Vec<ConventionException> = Vec::new();
    let mut modules: BTreeSet<String> = BTreeSet::new();
    for s in miner.population(g, partial) {
        match miner.conforms(&s, g) {
            Conformance::NotApplicable => {}
            Conformance::Conforms => {
                modules.insert(s.module.clone());
                judged.push((s, true));
            }
            Conformance::Violates { evidence } => {
                if let Some(reason) = exception_reason(&s, miner.id(), &generated, &configured) {
                    exceptions.push(ConventionException {
                        path: s.path.clone(),
                        reason: Some(format!("{reason}: {evidence}")),
                    });
                } else {
                    modules.insert(s.module.clone());
                    judged.push((s, false));
                }
            }
        }
    }
    if judged.is_empty() {
        return None;
    }
    let conforming = judged.iter().filter(|(_, ok)| *ok).count() as u32;
    let violations = judged.len() as u32 - conforming;
    let samples = judged.len() as u32;
    let cons = consistency(conforming, violations);
    let conf = confidence(cons, samples, modules.len() as u32);
    let convention = Convention {
        id: miner.id().to_owned(),
        rule: miner.rule().to_owned(),
        scope: narrow_scope(&judged),
        samples,
        violations,
        consistency: cons,
        confidence: conf,
        exceptions,
        source: ConventionSource::Inferred,
        computed_at: now,
        profile_version: PROFILE_VERSION,
    };
    metrics::convention_mined(miner.id(), is_enforceable(conf, samples));
    metrics::miner_duration(miner.id(), started.elapsed().as_secs_f64());
    Some(convention)
}
