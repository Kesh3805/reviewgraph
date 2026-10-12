//! `review eval`: developer tooling for the quality benchmark corpus (EVAL-001..EVAL-005).
//! Corpus data lives in `benchmarks/quality/`.

// Parts of the harness are consumed only by later commands (runner, reports).
#![allow(dead_code)]

pub mod build;
pub mod case;
pub mod category_compat;
pub mod matcher;

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum EvalAction {
    /// Validate every case under the cases directory (exit 1 on any invalid case).
    Validate {
        /// Directory holding `<case_id>/` subdirectories.
        #[arg(long, default_value = "../benchmarks/quality/cases")]
        cases: PathBuf,
    },
}

pub fn run(action: EvalAction) -> Result<()> {
    match action {
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
