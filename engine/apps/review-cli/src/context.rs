//! Resolves the repository root and `.review/` directory (CLI-001).

use std::path::{Path, PathBuf};

use crate::exit::{CliError, ExitCode};

pub struct Context {
    pub repo_root: PathBuf,
    pub review_dir: PathBuf,
}

/// Walk upward from `start` looking for a `.git` entry.
pub fn discover_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(Path::to_path_buf)
}

impl Context {
    pub fn resolve(repo: Option<&Path>) -> Result<Self, CliError> {
        let repo_root = match repo {
            Some(p) => p.to_path_buf(),
            None => {
                let cwd = std::env::current_dir()
                    .map_err(|e| CliError::new(ExitCode::Internal, e.to_string()))?;
                discover_repo_root(&cwd).unwrap_or(cwd)
            }
        };
        let review_dir = repo_root.join(".review");
        Ok(Self {
            repo_root,
            review_dir,
        })
    }

    pub fn is_initialized(&self) -> bool {
        self.review_dir.join("repository.json").is_file()
    }

    /// Fail with exit code 3 when `.review/` has not been initialized.
    pub fn require_initialized(&self) -> Result<(), CliError> {
        if self.is_initialized() {
            Ok(())
        } else {
            Err(CliError::not_initialized())
        }
    }
}
