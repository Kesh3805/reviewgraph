#![allow(dead_code)]

mod cli;
mod context;
mod contracts;
mod eval;
mod exit;
mod logging;
mod output;
mod semantic_cmd;

use clap::Parser;

use crate::cli::{Cli, Command, ContractsAction};
use crate::exit::{CliError, ExitCode};
use crate::output::{color_enabled, Output};

fn run(cli: Cli, out: &Output) -> Result<(), CliError> {
    let internal = |e: anyhow::Error| CliError::new(ExitCode::Internal, format!("{e:#}"));
    tracing::info!("review started");
    match cli.command {
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut cli::command(), "review", &mut std::io::stdout());
            Ok(())
        }
        Command::Contracts {
            action: ContractsAction::Export { out: dir },
        } => {
            let written = contracts::export(&dir).map_err(internal)?;
            let paths: Vec<String> = written.iter().map(|p| p.display().to_string()).collect();
            out.emit(
                "contracts export",
                serde_json::json!({ "written": paths }),
                vec![],
                &paths.join("\n"),
            )
            .map_err(internal)
        }
        Command::Semantic { action } => {
            let database_url = cli.database_url.as_deref().ok_or_else(|| {
                CliError::new(
                    ExitCode::Usage,
                    "--database-url or DATABASE_URL is required for semantic commands",
                )
            })?;
            semantic_cmd::run(database_url, action).map_err(internal)
        }
        Command::Eval { action } => eval::run(action).map_err(internal),
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let code = if e.use_stderr() {
                ExitCode::Usage.code()
            } else {
                0
            };
            let _ = e.print();
            std::process::exit(code);
        }
    };
    let guard = logging::init(cli.verbose, cli.quiet, cli.color);
    let out = Output::new(cli.format, color_enabled(cli.color));
    let result = run(cli, &out);
    // `process::exit` skips destructors: flush telemetry first.
    drop(guard);
    if let Err(err) = result {
        out.error(&err);
        std::process::exit(err.exit.code());
    }
}
