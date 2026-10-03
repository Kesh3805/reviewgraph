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

/// Canary strings planted in sensitive files. No output of the repository crate may contain one.
pub const CANARIES: &[&str] = &["RG_CANARY_5f1c", "RG_CANARY_ENV_77"];

/// A private copy of the `init-edge-cases` fixture with the content a plain-file fixture cannot
/// hold: ignored files, symlinks, binary and oversized files, sensitive files with canary
/// content, an LFS pointer, a non-UTF-8 file name and a case collision.
pub fn edge_case_tree() -> TempDir {
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    {
        let tmp = fixture_copy("init-edge-cases");
        let dir = tmp.path();
        // Files that the fixture's own ignore rules exclude.
        write_file(dir, "src/ignored-by-git.ts", "export const ignored = 1;\n");
        write_file(dir, "build-cache/x.js", "x\n");
        write_file(dir, "debug.log", "log\n");
        write_file(dir, "packages/a/local.ts", "export const local = 1;\n");
        write_file(dir, "packages/b/scratch.ts", "export const scratch = 1;\n");
        // Vendored directories.
        write_file(dir, "node_modules/pkg/index.js", "module.exports = 1;\n");
        write_file(
            dir,
            "nested/node_modules/pkg/index.js",
            "module.exports = 2;\n",
        );
        // Build output.
        write_file(dir, "dist/bundle.js", "var a = 1;\n");
        write_file(dir, "dist/types.d.ts", "export {};\n");
        write_file(dir, "coverage/lcov.info", "TN:\n");
        write_file(dir, "assets/app.min.js", "var a=1;\n");
        write_file(dir, "package-lock.json", "{\"lockfileVersion\": 3}\n");
        // Binary by extension and by NUL sniff.
        fs::write(dir.join("logo.png"), [0x89, b'P', b'N', b'G', 0, 1, 2]).expect("png");
        fs::write(dir.join("blob.dat"), [b'a', 0, b'b']).expect("dat");
        // LFS pointer.
        write_file(
            dir,
            "model.weights",
            "version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 12345\n",
        );
        // Oversized (2 MiB of text).
        write_file(dir, "big.txt", "x".repeat(2 * 1024 * 1024));
        // Sensitive files with canary content.
        write_file(
            dir,
            "gcs-key.json",
            "{\"private_key\": \"RG_CANARY_5f1c\"}\n",
        );
        write_file(dir, ".env.production", "SECRET=RG_CANARY_5f1c\n");
        write_file(dir, ".env.test", "TOKEN=RG_CANARY_ENV_77\n");
        write_file(dir, "certs/server.pem", "-----BEGIN-----\nRG_CANARY_5f1c\n");
        // Case collision.
        write_file(dir, "Readme.md", "# one\n");
        write_file(dir, "README.md", "# two\n");
        // Non-UTF-8 file name.
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let name = OsStr::from_bytes(b"bad-\xff-name.ts");
            fs::write(dir.join(name), "export {};\n").expect("non utf8");
        }
        // Symlinks: one inside the tree, one escaping it.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("src/app.ts", dir.join("link-in.ts")).expect("symlink");
            std::os::unix::fs::symlink("/etc/passwd", dir.join("link-out")).expect("symlink");
            std::os::unix::fs::symlink("../../outside.txt", dir.join("link-up")).expect("symlink");
        }
        tmp
    }
}

/// Like [`fixture_copy`], but the copy lives in `<tmp>/<name>` so the root directory name is
/// stable (some facts include the repository directory name).
pub fn fixture_copy_named(name: &str) -> (TempDir, PathBuf) {
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    {
        let src = fixture_repo(name);
        let tmp = tempfile::tempdir().expect("tempdir");
        let dst = tmp.path().join(name);
        fs::create_dir_all(&dst).expect("mkdir");
        copy_dir(&src, &dst).expect("copy fixture");
        (tmp, dst)
    }
}
