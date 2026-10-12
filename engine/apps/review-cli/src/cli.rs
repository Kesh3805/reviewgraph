//! clap command tree (CLI-001). Subcommand logic lands in CLI-002..CLI-013.

use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

use crate::output::Format;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum StoreKind {
    Auto,
    Local,
    Postgres,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Parser)]
#[command(name = "review", version, about = "ReviewGraph command line")]
pub struct Cli {
    /// Repository path (default: discover upward for `.git`).
    #[arg(long, global = true)]
    pub repo: Option<PathBuf>,
    /// Output format.
    #[arg(long, global = true, value_enum, default_value = "human")]
    pub format: Format,
    /// Graph store backend.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub store: StoreKind,
    /// Postgres connection string (only with `--store postgres`).
    #[arg(long, global = true, env = "DATABASE_URL", hide_env_values = true)]
    pub database_url: Option<String>,
    /// Colored output.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub color: ColorMode,
    /// Increase log verbosity (stderr).
    #[arg(short = 'v', global = true, action = ArgAction::Count)]
    pub verbose: u8,
    /// Decrease log verbosity.
    #[arg(short = 'q', global = true, action = ArgAction::Count)]
    pub quiet: u8,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Contract (Rust -> JSON Schema -> TypeScript) tooling.
    Contracts {
        #[command(subcommand)]
        action: ContractsAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum ContractsAction {
    /// Write JSON Schemas for every registered contract type.
    Export {
        /// Output directory (default: ../packages/contracts/schemas, relative to engine/).
        #[arg(long, default_value = "../packages/contracts/schemas")]
        out: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_global_flags() {
        let cli =
            Cli::try_parse_from(["review", "--format", "json", "contracts", "export"]).unwrap();
        assert_eq!(cli.format, Format::Json);
    }
}
