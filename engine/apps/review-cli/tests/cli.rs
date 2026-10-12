#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! CLI-001 black-box tests: streams, exit codes, completions.

mod common;

use common::{run, stderr, stdout};

#[test]
fn logs_go_to_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("schemas");
    let output = run(&[
        "-v",
        "--format",
        "json",
        "contracts",
        "export",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);
    // Exactly one JSON document on stdout, and no log line in it.
    let doc: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["command"], "contracts export");
    assert!(!out.contains("review started"));
    assert!(
        stderr(&output).contains("review started"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(out_dir.join("CliOutputV1.schema.json").is_file());
}

#[test]
fn quiet_run_prints_no_logs() {
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("schemas");
    let output = run(&["contracts", "export", "--out", out_dir.to_str().unwrap()]);
    assert!(output.status.success());
    assert!(stderr(&output).is_empty(), "stderr: {}", stderr(&output));
}

#[test]
fn usage_error_exits_2() {
    let output = run(&["--no-such-flag"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn version_flag_works() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("review "));
}

#[test]
fn completions_generate_for_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = run(&["completions", shell]);
        assert!(output.status.success(), "{shell}: {}", stderr(&output));
        assert!(
            stdout(&output).contains("review"),
            "{shell} script is empty"
        );
    }
}
