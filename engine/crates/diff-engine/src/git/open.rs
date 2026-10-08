//! Repository opening with an isolated gix configuration (DIFF-001).

use std::path::{Path, PathBuf};

use super::{GitError, GitRepo, ReadLimits};

impl GitRepo {
    /// Open a repository (normal or bare) with an isolated configuration: no system, global or
    /// user config, no includes, no hooks, filters or attributes, and no git environment
    /// variables. Rejects paths whose `.git` resolves outside the opened root through a symlink.
    pub fn open(path: &Path, limits: ReadLimits) -> Result<Self, GitError> {
        open_inner(path, limits)
    }

    /// Open a bare mirror checkout with the same isolation as [`GitRepo::open`].
    pub fn open_mirror(path: &Path, limits: ReadLimits) -> Result<Self, GitError> {
        open_inner(path, limits)
    }
}

fn open_inner(path: &Path, limits: ReadLimits) -> Result<GitRepo, GitError> {
    let options =
        gix::open::Options::isolated().config_overrides(["core.autocrlf=false", "core.fsmonitor="]);
    let inner = options.open(path).map_err(|e| open_error(path, e))?;
    let root = canonical_root(path)?;
    let git_dir = std::fs::canonicalize(inner.git_dir()).map_err(|e| GitError::Io {
        path: inner.git_dir().to_path_buf(),
        source: e,
    })?;
    if !git_dir.starts_with(&root) {
        return Err(GitError::SymlinkEscape(inner.git_dir().to_path_buf()));
    }
    check_alternates(&git_dir, &root)?;
    Ok(GitRepo { inner, limits })
}

/// Object alternates are honoured only when their resolved location stays inside the opened
/// root, so a hostile `.git` cannot pull objects in from elsewhere on the host.
fn check_alternates(git_dir: &Path, root: &Path) -> Result<(), GitError> {
    let path = git_dir.join("objects").join("info").join("alternates");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let listed = Path::new(line);
        let full = if listed.is_absolute() {
            listed.to_path_buf()
        } else {
            git_dir.join("objects").join(listed)
        };
        let resolved = std::fs::canonicalize(&full).unwrap_or(full);
        if !resolved.starts_with(root) {
            return Err(GitError::SymlinkEscape(resolved));
        }
    }
    Ok(())
}

fn canonical_root(path: &Path) -> Result<PathBuf, GitError> {
    std::fs::canonicalize(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GitError::NotARepository(path.to_path_buf())
        } else {
            GitError::Io {
                path: path.to_path_buf(),
                source: e,
            }
        }
    })
}

fn open_error(path: &Path, err: gix::Error) -> GitError {
    let message = err.to_string();
    if !looks_like_repo(path) || message.to_ascii_lowercase().contains("not a repository") {
        GitError::NotARepository(path.to_path_buf())
    } else {
        GitError::Gix(message)
    }
}

fn looks_like_repo(path: &Path) -> bool {
    path.join(".git").exists() || (path.join("HEAD").is_file() && path.join("objects").is_dir())
}
