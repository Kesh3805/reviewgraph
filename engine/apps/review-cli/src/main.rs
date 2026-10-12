#![allow(dead_code)]

mod cli;
mod context;
mod contracts;
mod exit;
mod output;

use clap::Parser;

use crate::cli::{Cli, Command, ContractsAction};
use crate::exit::{CliError, ExitCode};
use crate::output::Output;

fn run(cli: Cli, out: &Output) -> Result<(), CliError> {
    let internal = |e: anyhow::Error| CliError::new(ExitCode::Internal, format!("{e:#}"));
    match cli.command {
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
    let out = Output::new(cli.format);
    if let Err(err) = run(cli, &out) {
        out.error(&err);
        std::process::exit(err.exit.code());
    }
}
