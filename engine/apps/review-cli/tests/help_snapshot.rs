#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! CLI-001: the top-level `--help` output is snapshot-tested so flag renames are deliberate.

use std::process::Command;

fn help(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_review"))
        .args(args)
        .env("NO_COLOR", "1")
        .env_remove("DATABASE_URL")
        .env_remove("COLUMNS")
        .output()
        .unwrap();
    assert!(output.status.success(), "review {args:?} failed");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn help_snapshot() {
    insta::assert_snapshot!("help", help(&["--help"]));
}
