//! Clone credential hygiene for workers (SEC-005).
//!
//! A worker receives a short-lived, single-repository installation token from the API broker and
//! hands it to `git` for one checkout. The rules this module encodes:
//!
//! - the token never appears on a command line (visible in process listings): it travels as an
//!   `http.extraHeader` through the child's environment (`GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/
//!   `GIT_CONFIG_VALUE_n`, git 2.31+), which git does not persist;
//! - remote URLs never carry userinfo;
//! - interactive prompts and credential helpers are disabled, so nothing is written to a helper
//!   store;
//! - after checkout, [`assert_no_credentials_in_git_config`] proves `.git/config` holds no
//!   credential.
//!
//! This module builds the environment only; spawning `git` is the worker's job.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// An installation token for one clone. `Debug` never prints the value and there is no
/// `Display`.
pub struct CloneToken(String);

impl CloneToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The raw value. Only [`credential_env`] needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CloneToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CloneToken(«redacted»)")
    }
}

/// Environment variables that authenticate one `git` child process. `Debug` lists the variable
/// names only.
#[derive(Clone)]
pub struct GitCredentialEnv {
    vars: Vec<(&'static str, String)>,
}

impl fmt::Debug for GitCredentialEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitCredentialEnv")
            .field(
                "vars",
                &self.vars.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl GitCredentialEnv {
    /// `(name, value)` pairs for the child environment.
    pub fn vars(&self) -> impl Iterator<Item = (&'static str, &str)> {
        self.vars.iter().map(|(k, v)| (*k, v.as_str()))
    }

    /// Adds the variables to a command's environment (never to its arguments).
    pub fn apply(&self, command: &mut Command) {
        for (k, v) in &self.vars {
            command.env(k, v);
        }
    }
}

/// Builds the child-process environment that authenticates git over HTTPS with
/// `Authorization: Basic base64(username:token)`.
pub fn credential_env(username: &str, token: &CloneToken) -> GitCredentialEnv {
    let basic = base64(format!("{username}:{}", token.expose()).as_bytes());
    GitCredentialEnv {
        vars: vec![
            ("GIT_TERMINAL_PROMPT", "0".to_owned()),
            // An empty askpass and helper list: git never prompts and never stores credentials.
            ("GIT_ASKPASS", String::new()),
            ("GIT_CONFIG_NOSYSTEM", "1".to_owned()),
            ("GIT_CONFIG_COUNT", "2".to_owned()),
            ("GIT_CONFIG_KEY_0", "credential.helper".to_owned()),
            ("GIT_CONFIG_VALUE_0", String::new()),
            ("GIT_CONFIG_KEY_1", "http.extraHeader".to_owned()),
            (
                "GIT_CONFIG_VALUE_1",
                format!("Authorization: Basic {basic}"),
            ),
        ],
    }
}

/// Why a remote or a checkout fails the credential rules.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CredentialHygieneError {
    #[error("remote URL carries credentials (userinfo); use credential_env instead")]
    UserinfoInRemote,
    #[error("git config at {} contains a credential ({reason})", path.display())]
    CredentialInConfig { path: PathBuf, reason: &'static str },
    #[error("git config at {} could not be read", .0.display())]
    Unreadable(PathBuf),
}

/// Rejects `scheme://user[:pass]@host/...` remotes: a token in the URL ends up in `.git/config`
/// and in process listings.
pub fn ensure_credential_free_remote(url: &str) -> Result<(), CredentialHygieneError> {
    if let Some((_, rest)) = url.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if authority.contains('@') {
            return Err(CredentialHygieneError::UserinfoInRemote);
        }
    }
    Ok(())
}

/// Arguments for a credential-free `git clone`; the caller adds [`credential_env`] to the
/// environment. The remote is validated first.
pub fn clone_args(remote_url: &str, dest: &Path) -> Result<Vec<String>, CredentialHygieneError> {
    ensure_credential_free_remote(remote_url)?;
    Ok(vec![
        "clone".to_owned(),
        "--quiet".to_owned(),
        "--no-tags".to_owned(),
        "--".to_owned(),
        remote_url.to_owned(),
        dest.to_string_lossy().into_owned(),
    ])
}

/// Post-checkout assertion: the repository's `.git/config` (or `config` of a bare repository)
/// contains neither the token, nor an `extraheader`/`authorization` entry, nor a URL with
/// userinfo.
pub fn assert_no_credentials_in_git_config(
    repo: &Path,
    token: &CloneToken,
) -> Result<(), CredentialHygieneError> {
    let candidates = [repo.join(".git").join("config"), repo.join("config")];
    let Some(path) = candidates.into_iter().find(|p| p.is_file()) else {
        return Err(CredentialHygieneError::Unreadable(repo.join(".git/config")));
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|_| CredentialHygieneError::Unreadable(path.clone()))?;
    let lower = text.to_ascii_lowercase();
    let reason = if !token.expose().is_empty() && text.contains(token.expose()) {
        Some("token value")
    } else if lower.contains("extraheader") {
        Some("http.extraHeader entry")
    } else if lower.contains("authorization") {
        Some("authorization entry")
    } else if text
        .lines()
        .filter(|l| l.trim_start().to_ascii_lowercase().starts_with("url"))
        .any(|l| {
            l.split_once('=')
                .is_some_and(|(_, v)| ensure_credential_free_remote(v.trim()).is_err())
        })
    {
        Some("remote URL with userinfo")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(CredentialHygieneError::CredentialInConfig { path, reason }),
        None => Ok(()),
    }
}

/// Standard base64 with padding (no extra dependency for one header).
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = |shift: u32| char::from(TABLE[((n >> shift) & 63) as usize]);
        out.push(idx(18));
        out.push(idx(12));
        out.push(if chunk.len() > 1 { idx(6) } else { '=' });
        out.push(if chunk.len() > 2 { idx(0) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"x-access-token:abc"), "eC1hY2Nlc3MtdG9rZW46YWJj");
    }
}
