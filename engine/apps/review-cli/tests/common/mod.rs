//! Shared helpers for the CLI integration tests: run the real binary, build throwaway git
//! repositories.

#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Output};

/// A `review` invocation with a clean environment (no color, no ambient database or tokens).
pub fn review(args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_review"));
    cmd.args(args)
        .env("NO_COLOR", "1")
        .env_remove("DATABASE_URL")
        .env_remove("RUST_LOG")
        .env_remove("REVIEW_OTEL")
        .env_remove("GITHUB_TOKEN");
    cmd
}

pub fn run(args: &[&str]) -> Output {
    review(args).output().unwrap()
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Runs `git` in `dir` with a fixed identity and no ambient configuration.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Writes `files` (path, contents) into `dir`, creating parent directories.
pub fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (path, contents) in files {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, contents).unwrap();
    }
}

/// A repository with one commit holding `files`.
pub fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write_files(dir.path(), files);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    dir
}

/// Adds a commit that overwrites `files`.
pub fn commit_files(dir: &Path, files: &[(&str, &str)], message: &str) {
    write_files(dir, files);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}
