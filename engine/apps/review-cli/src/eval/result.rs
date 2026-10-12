//! One JSONL row per (case, repetition): the runner's raw result (EVAL-003), the input of the
//! metrics (EVAL-004) and reports (EVAL-005).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::case::CaseClass;
use super::matcher::{MatchResult, ProducedFinding};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub model_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd_micros: u64,
}

/// Structured-output outcomes of the case's model calls (transport failures excluded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredStats {
    pub success: u64,
    pub failures_after_repair: u64,
    /// Failures by task (`correctness_review`, ...).
    #[serde(default)]
    pub failures_by_task: BTreeMap<String, u64>,
}

/// Usage of one (tier, provider, model) within a case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallStat {
    pub task: String,
    pub tier: String,
    pub provider: String,
    pub model: String,
    pub routing_table_hash: String,
    pub prompt_version: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub run_id: String,
    pub config_name: String,
    pub case_id: String,
    pub repetition: u32,
    pub classes: Vec<CaseClass>,
    /// Every reviewer the case requires was enabled in the run.
    pub required_reviewers_enabled: bool,
    pub synthetic_fixtures: bool,
    /// `replay_miss`, `panic`, `no_context`, ... A case with an error is excluded from metrics.
    pub case_error: Option<String>,
    /// Request hashes that missed under replay (actionable: author or record fixtures).
    #[serde(default)]
    pub replay_misses: Vec<String>,
    /// A live provider failure degraded the case (counts against verification success).
    #[serde(default)]
    pub degraded: bool,
    pub findings: Vec<ProducedFinding>,
    pub matches: Option<MatchResult>,
    pub usage: UsageTotals,
    pub structured: StructuredStats,
    /// Wall time start → verified, in milliseconds.
    pub latency_ms: u64,
    /// Symbol ids present in any context package of the case (context-miss diagnostic).
    pub context_symbols: Vec<String>,
    pub calls: Vec<CallStat>,
}

impl CaseResult {
    pub fn is_example(&self) -> bool {
        self.case_id.starts_with("example-")
    }
}
