//! Output abstraction: human tables or one JSON document on stdout (CLI-001).
//!
//! Logs never use this module: they go to stderr (see `logging`).

use std::io::{IsTerminal, Write};

use clap::ValueEnum;
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{ContentArrangement, Table};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::ColorMode;
use crate::exit::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

// The versioned JSON envelope every command prints with `--format json` (`cli-output.v1`).
// Exported to contracts as `CliOutputV1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CliOutputV1 {
    pub version: u32,
    pub command: String,
    pub data: Value,
    pub warnings: Vec<String>,
}

/// Terminal styling for a human-readable word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Dim,
}

#[derive(Debug, Clone, Copy)]
pub struct Output {
    pub format: Format,
    pub color: bool,
}

/// Whether ANSI colors are wanted for stdout.
pub fn color_enabled(mode: ColorMode) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
        }
    }
}

/// Renders rows as a table with a header line. Plain UTF-8 borders, no terminal probing, so
/// the output is identical on every host.
pub fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Disabled)
        .set_header(headers.iter().map(|h| (*h).to_owned()).collect::<Vec<_>>());
    for row in rows {
        table.add_row(row);
    }
    table.to_string()
}

impl Output {
    pub fn new(format: Format, color: bool) -> Self {
        Self { format, color }
    }

    /// Wraps `text` in an ANSI style when colors are on and the format is human.
    pub fn paint(&self, text: &str, tone: Tone) -> String {
        if !self.color || self.format != Format::Human {
            return text.to_owned();
        }
        let code = match tone {
            Tone::Good => "32",
            Tone::Warn => "33",
            Tone::Bad => "31",
            Tone::Dim => "2",
        };
        format!("\u{1b}[{code}m{text}\u{1b}[0m")
    }

    pub fn is_json(&self) -> bool {
        self.format == Format::Json
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
                let env = CliOutputV1 {
                    version: 1,
                    command: command.to_owned(),
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
                let mut error = serde_json::json!({
                    "code": err.exit.name(),
                    "message": err.message,
                });
                if let (Some(hint), Some(map)) = (&err.hint, error.as_object_mut()) {
                    map.insert("hint".to_owned(), Value::String(hint.clone()));
                }
                let doc = serde_json::json!({ "error": error });
                println!("{doc}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> Value {
        serde_json::to_value(CliOutputV1 {
            version: 1,
            command: "status".to_owned(),
            data: serde_json::json!({"a": 1}),
            warnings: vec!["w".to_owned()],
        })
        .unwrap()
    }

    #[test]
    fn json_envelope_shape() {
        let v = envelope();
        assert_eq!(v["version"], 1);
        assert_eq!(v["command"], "status");
        assert!(v["warnings"].is_array());
    }

    /// Checks the envelope against the schema that `contracts export` publishes: required keys,
    /// no extra keys (`additionalProperties: false`) and the declared primitive types.
    #[test]
    fn json_envelope_schema_valid() {
        let schema = serde_json::to_value(schemars::schema_for!(CliOutputV1)).unwrap();
        let doc = envelope();
        let doc_obj = doc.as_object().unwrap();
        let props = schema["properties"].as_object().unwrap();
        for key in schema["required"].as_array().unwrap() {
            let key = key.as_str().unwrap();
            assert!(doc_obj.contains_key(key), "missing required key {key}");
        }
        assert_eq!(schema["additionalProperties"], Value::Bool(false));
        for key in doc_obj.keys() {
            assert!(props.contains_key(key), "unexpected key {key}");
        }
        assert_eq!(props["version"]["type"], "integer");
        assert!(doc["version"].is_u64());
        assert_eq!(props["command"]["type"], "string");
        assert!(doc["command"].is_string());
        assert_eq!(props["warnings"]["type"], "array");
        assert_eq!(props["warnings"]["items"]["type"], "string");
        assert!(doc["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(Value::is_string));
        // The envelope round-trips through its own type.
        let back: CliOutputV1 = serde_json::from_value(doc).unwrap();
        assert_eq!(back.command, "status");
    }

    #[test]
    fn table_renders_headers_and_rows() {
        let text = table(
            &["name", "kind"],
            vec![vec!["a".to_owned(), "function".to_owned()]],
        );
        assert!(text.contains("name"));
        assert!(text.contains("function"));
    }

    #[test]
    fn paint_is_plain_without_color() {
        let out = Output::new(Format::Human, false);
        assert_eq!(out.paint("ok", Tone::Good), "ok");
        let out = Output::new(Format::Human, true);
        assert!(out.paint("ok", Tone::Good).contains("\u{1b}[32m"));
    }
}
