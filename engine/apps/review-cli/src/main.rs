mod contracts;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "review", about = "ReviewGraph command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Contract (Rust -> JSON Schema -> TypeScript) tooling.
    Contracts {
        #[command(subcommand)]
        action: ContractsAction,
    },
}

#[derive(Debug, Subcommand)]
enum ContractsAction {
    /// Write JSON Schemas for every registered contract type.
    Export {
        /// Output directory (default: ../packages/contracts/schemas, relative to engine/).
        #[arg(long, default_value = "../packages/contracts/schemas")]
        out: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let _telemetry = telemetry::init(telemetry::TelemetryConfig::from_env_with_format(
        "review-cli",
        telemetry::LogFormat::Pretty,
    )?)?;
    match Cli::parse().command {
        Command::Contracts {
            action: ContractsAction::Export { out },
        } => {
            let written = contracts::export(&out)?;
            for path in written {
                println!("{}", path.display());
            }
            Ok(())
        }
    }
}
