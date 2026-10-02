//! Test helpers shared by crate integration tests (INIT-001).
//!
//! This crate is a dev-dependency only. It lives under `engine/tools/` rather than
//! `engine/crates/` so it is not a library crate in the dependency-direction rules
//! (target-architecture §2.1). It shells out to `git` and `bash`, which production crates must
//! never do.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

pub use tempfile::TempDir;

/// Root of the reviewgraph repository (the directory that holds `fixtures/`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .components()
        .fold(PathBuf::new(), |mut acc, c| {
            use std::path::Component;
            match c {
                Component::ParentDir => {
                    acc.pop();
                }
                Component::CurDir => {}
                other => acc.push(other.as_os_str()),
            }
            acc
        })
}

fn build_cache() -> &'static Mutex<HashMap<String, PathBuf>> {
    static CACHE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Materializes the named fixture with `fixtures/build.sh` once per test process and returns the
/// built repository path. The returned directory is shared: tests must not mutate it. Use
/// [`fixture_copy`] for tests that need to change the repository.
///
/// Panics (test helper) when the build fails.
pub fn fixture_repo(name: &str) -> PathBuf {
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    {
        let mut cache = build_cache().lock().unwrap();
        if let Some(path) = cache.get(name) {
            return path.clone();
        }
        let root = repo_root();
        let out = root.join("fixtures/.build/repos");
        let status = Command::new("bash")
            .arg(root.join("fixtures/build.sh"))
            .arg(name)
            .arg("--out")
            .arg(&out)
            .output()
            .expect("run fixtures/build.sh");
        assert!(
            status.status.success(),
            "fixture build failed for {name}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        let path = out.join(name);
        cache.insert(name.to_owned(), path.clone());
        path
    }
}

/// A private, writable copy of a built fixture (including its `.git` directory).
pub fn fixture_copy(name: &str) -> TempDir {
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    {
        let src = fixture_repo(name);
        let tmp = tempfile::tempdir().expect("tempdir");
        copy_dir(&src, tmp.path()).expect("copy fixture");
        tmp
    }
}

/// Recursively copies `src` into the existing directory `dst`.
pub fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&to)?;
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Runs `git` in `dir` with user and system configuration disabled and a fixed identity.
/// Returns trimmed stdout; panics (test helper) on a non-zero exit.
pub fn git(dir: &Path, args: &[&str]) -> String {
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    {
        let out = git_command(dir, args).output().expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }
}

fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .args(["-c", "protocol.file.allow=always"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
    cmd
}

/// `git init -b main` in a fresh temp directory (an unborn HEAD).
pub fn empty_repo() -> TempDir {
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    {
        let tmp = tempfile::tempdir().expect("tempdir");
        git(tmp.path(), &["init", "-q", "-b", "main"]);
        tmp
    }
}

/// Adds an `origin` remote and the remote-tracking refs a clone would have, so
/// `refs/remotes/origin/HEAD` points at `origin/<branch>`.
pub fn add_origin(dir: &Path, url: &str, branch: &str) {
    git(dir, &["remote", "add", "origin", url]);
    git(
        dir,
        &[
            "update-ref",
            &format!("refs/remotes/origin/{branch}"),
            &format!("refs/heads/{branch}"),
        ],
    );
    git(
        dir,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            &format!("refs/remotes/origin/{branch}"),
        ],
    );
}

/// Writes `contents` to `dir/rel`, creating parent directories.
pub fn write_file(dir: &Path, rel: &str, contents: impl AsRef<[u8]>) {
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(path, contents).expect("write file");
    }
}
