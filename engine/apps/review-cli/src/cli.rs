//! clap command tree (CLI-001). Subcommand logic lives in `commands/`.

use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

use crate::output::Format;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StoreKind {
    Auto,
    Local,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Parser)]
#[command(name = "review", version, about = "ReviewGraph command line")]
pub struct Cli {
    /// Repository path (default: discover upward for `.git`).
    #[arg(long, alias = "repository", global = true)]
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
    /// Generate shell completions on stdout.
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Contract (Rust -> JSON Schema -> TypeScript) tooling.
    Contracts {
        #[command(subcommand)]
        action: ContractsAction,
    },
    /// Semantic (vector) collection registry and cut-over.
    Semantic {
        #[command(subcommand)]
        action: crate::semantic_cmd::SemanticAction,
    },
    /// Quality benchmark tooling (developer use).
    Eval {
        #[command(subcommand)]
        action: crate::eval::EvalAction,
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

/// The command tree, for completions and help snapshots.
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        command().debug_assert();
    }

    #[test]
    fn parses_global_flags() {
        let cli =
            Cli::try_parse_from(["review", "--format", "json", "contracts", "export"]).unwrap();
        assert_eq!(cli.format, Format::Json);
    }

    #[test]
    fn repository_is_an_alias_of_repo() {
        let cli = Cli::try_parse_from([
            "review",
            "completions",
            "bash",
            "--repository",
            "some/where",
        ])
        .unwrap();
        assert_eq!(cli.repo, Some(PathBuf::from("some/where")));
    }

    #[test]
    fn completions_generate() {
        use clap_complete::Shell;
        for shell in [
            Shell::Bash,
            Shell::Zsh,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Elvish,
        ] {
            let mut buf: Vec<u8> = Vec::new();
            clap_complete::generate(shell, &mut command(), "review", &mut buf);
            let text = String::from_utf8(buf).unwrap();
            assert!(text.contains("review"), "{shell:?} script is empty");
            assert!(
                text.contains("completions"),
                "{shell:?} misses a subcommand"
            );
        }
    }
}
