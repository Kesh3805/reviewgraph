//! `review eval`: developer tooling for the quality benchmark corpus (EVAL-001..EVAL-005).
//! Corpus data lives in `benchmarks/quality/`.

// Parts of the harness are consumed only by later commands (runner, reports).
#![allow(dead_code)]

pub mod build;
pub mod case;
pub mod category_compat;
pub mod matcher;
pub mod metrics;

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum EvalAction {
    /// Compute metrics from a results JSONL; exit 2 on regression against `--baseline`.
    Metrics {
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        baseline: Option<PathBuf>,
    },
    /// Validate every case under the cases directory (exit 1 on any invalid case).
    Validate {
        /// Directory holding `<case_id>/` subdirectories.
        #[arg(long, default_value = "../benchmarks/quality/cases")]
        cases: PathBuf,
    },
}

pub fn run(action: EvalAction) -> Result<()> {
    match action {
        EvalAction::Metrics { results, baseline } => {
            let rows = metrics::parse_results(&std::fs::read_to_string(&results)?)?;
            let current = metrics::compute(&rows);
            println!("{}", serde_json::to_string_pretty(&current)?);
            if let Some(path) = baseline {
                let base: metrics::Metrics = serde_json::from_str(&std::fs::read_to_string(path)?)?;
                let failed = metrics::regressions(&current, &base);
                if !failed.is_empty() {
                    for f in &failed {
                        eprintln!("REGRESSION {f}");
                    }
                    std::process::exit(2);
                }
            }
            Ok(())
        }
        EvalAction::Validate { cases } => {
            let dirs = case::list_case_dirs(&cases)?;
            let mut failures = 0usize;
            for dir in &dirs {
                if let Err(err) = case::load_case(dir) {
                    failures += 1;
                    eprintln!("INVALID {}: {err:#}", dir.display());
                }
            }
            if failures > 0 {
                bail!("{failures} invalid case(s)");
            }
            println!(
                "{} case(s) valid; corpus_hash={}",
                dirs.len(),
                case::corpus_hash(&dirs)?
            );
            Ok(())
        }
    }
}
