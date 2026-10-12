//! The miner registry (PROF-003): which miners run, their versions (which enter
//! `profile_version`), and the parallel run over an immutable graph.

use std::time::Instant;

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use review_core::version::ProfileVersion;

use super::builtin::api::ApiMiner;
use super::builtin::controller_repository::ControllerRepositoryMiner;
use super::builtin::error_hierarchy::ErrorHierarchyMiner;
use super::builtin::guard_usage::GuardUsageMiner;
use super::builtin::layer_dependency::{self, LayerDependencyMiner};
use super::builtin::queues::QueuesMiner;
use super::builtin::testing::TestingMiner;
use super::builtin::transaction_wrapper::TransactionWrapperMiner;
use super::miner::{mine, ConventionMiner, MiningContext};
use crate::model::{Convention, RepositoryProfile};

/// Layer-dependency conventions below this consistency are noise ("services usually call
/// repositories" is not a convention against it) and are not emitted.
const LAYER_DEPENDENCY_MIN_CONSISTENCY: f32 = 0.5;

pub struct ConventionRegistry {
    miners: Vec<Box<dyn ConventionMiner>>,
    /// Also mine the observed layer matrix (PROF-002).
    layer_dependencies: bool,
}

impl std::fmt::Debug for ConventionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConventionRegistry")
            .field("miners", &self.versions())
            .field("layer_dependencies", &self.layer_dependencies)
            .finish()
    }
}

impl Default for ConventionRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ConventionRegistry {
    pub fn empty() -> Self {
        Self {
            miners: Vec::new(),
            layer_dependencies: false,
        }
    }

    /// The PROF-002, PROF-004 and PROF-005 miners.
    pub fn builtin() -> Self {
        let mut registry = Self::empty();
        registry.layer_dependencies = true;
        registry.register(Box::new(ControllerRepositoryMiner));
        registry.register(Box::new(TransactionWrapperMiner));
        registry.register(Box::new(ErrorHierarchyMiner));
        registry.register(Box::new(GuardUsageMiner));
        registry.register(Box::new(TestingMiner));
        registry.register(Box::new(ApiMiner));
        registry.register(Box::new(QueuesMiner));
        registry
    }

    pub fn register(&mut self, miner: Box<dyn ConventionMiner>) {
        self.miners.push(miner);
    }

    /// `(miner id, version)` of every miner, sorted; the input of `profile_version`.
    pub fn versions(&self) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = self
            .miners
            .iter()
            .map(|m| (m.id().to_owned(), m.version()))
            .collect();
        if self.layer_dependencies {
            out.push(("layer_dependency".to_owned(), layer_dependency::VERSION));
        }
        out.sort();
        out
    }

    /// Runs every miner in parallel. A miner whose population is empty emits nothing; the
    /// result is sorted by convention id.
    pub fn mine_all(
        &self,
        cx: &MiningContext<'_>,
        profile_version: ProfileVersion,
        computed_at: DateTime<Utc>,
    ) -> Vec<Convention> {
        let layer_miners: Vec<LayerDependencyMiner> = if self.layer_dependencies {
            LayerDependencyMiner::for_present_roles(cx)
        } else {
            Vec::new()
        };
        let mut all: Vec<&dyn ConventionMiner> = self.miners.iter().map(|m| m.as_ref()).collect();
        all.extend(layer_miners.iter().map(|m| m as &dyn ConventionMiner));
        let mut conventions: Vec<Convention> = all
            .par_iter()
            .filter_map(|miner| {
                let started = Instant::now();
                let convention = mine(*miner, cx, profile_version, computed_at);
                let label = metric_label(miner.id());
                crate::metrics::miner_duration(label, started.elapsed().as_secs_f64());
                let convention = convention?;
                if convention.id.starts_with("layer_dependency:")
                    && convention.consistency < LAYER_DEPENDENCY_MIN_CONSISTENCY
                {
                    return None;
                }
                crate::metrics::convention_mined(label, convention.enforceable);
                Some(convention)
            })
            .collect();
        conventions.sort_by(|a, b| a.id.cmp(&b.id));
        conventions
    }

    /// Lets every miner fill the profile sections it owns, in registration order.
    pub fn contribute_all(&self, cx: &MiningContext<'_>, profile: &mut RepositoryProfile) {
        for miner in &self.miners {
            miner.contribute(cx, profile);
        }
    }
}

/// A bounded metric label: built-in ids are static, the layer family shares one label.
fn metric_label(id: &str) -> &'static str {
    use super::builtin::{
        api, controller_repository, error_hierarchy, guard_usage, queues, testing,
        transaction_wrapper,
    };
    for known in [
        api::ID,
        controller_repository::ID,
        error_hierarchy::ID,
        guard_usage::ID,
        queues::ID,
        testing::ID,
        transaction_wrapper::ID,
    ] {
        if id == known {
            return known;
        }
    }
    if id.starts_with("layer_dependency:") {
        "layer_dependency"
    } else {
        "custom"
    }
}
