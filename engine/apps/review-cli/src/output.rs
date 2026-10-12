//! Output abstraction: human text or one JSON document on stdout (CLI-001).

use std::io::Write;

use clap::ValueEnum;
use serde::Serialize;
use serde_json::Value;

use crate::exit::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// Versioned JSON envelope (`cli-output.v1`).
#[derive(Debug, Serialize)]
pub struct Envelope<'a> {
    pub version: u32,
    pub command: &'a str,
    pub data: Value,
    pub warnings: Vec<String>,
}

pub struct Output {
    pub format: Format,
}

impl Output {
    pub fn new(format: Format) -> Self {
        Self { format }
    }

    /// Emit a successful result. Human mode prints `human`; JSON mode prints the envelope.
    pub fn emit(
        &self,
        command: &str,
        data: Value,
        warnings: Vec<String>,
        human: &str,
    ) -> anyhow::Result<()> {
        let mut out = std::io::stdout().lock();
        match self.format {
            Format::Human => {
                writeln!(out, "{human}")?;
                for w in &warnings {
                    eprintln!("warning: {w}");
                }
            }
            Format::Json => {
                let env = Envelope {
                    version: 1,
                    command,
                    data,
                    warnings,
                };
                writeln!(out, "{}", serde_json::to_string(&env)?)?;
            }
        }
        Ok(())
    }

    /// Emit an error in the active format. Human errors go to stderr.
    pub fn error(&self, err: &CliError) {
        match self.format {
            Format::Human => {
                eprintln!("error: {}", err.message);
                if let Some(hint) = &err.hint {
                    eprintln!("hint: {hint}");
                }
            }
            Format::Json => {
                let doc = serde_json::json!({
                    "error": { "code": err.exit.name(), "message": err.message }
                });
                println!("{doc}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_envelope_shape() {
        let env = Envelope {
            version: 1,
            command: "status",
            data: serde_json::json!({"a": 1}),
            warnings: vec![],
        };
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["command"], "status");
        assert!(v["warnings"].is_array());
    }
}
