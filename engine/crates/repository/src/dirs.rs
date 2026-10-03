//! `RepoDir`: a repository-relative directory that may be the repository root.
//!
//! `RepoPath` forbids the empty path, but many facts are scoped to "the root directory" (a
//! workspace root, the directory a tsconfig lives in). `RepoDir` is the string `""` for the root
//! and otherwise a validated `RepoPath`.

use std::fmt;

use review_core::location::RepoPath;
use review_core::CoreError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema, Default)]
#[serde(transparent)]
pub struct RepoDir(String);

impl RepoDir {
    pub fn root() -> Self {
        Self(String::new())
    }

    /// `""` is the root; anything else must be a valid [`RepoPath`].
    pub fn new(s: impl Into<String>) -> Result<Self, CoreError> {
        let s = s.into();
        if s.is_empty() {
            return Ok(Self(s));
        }
        RepoPath::new(s).map(Self::from)
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The directory containing `path` (the root for top-level files).
    pub fn parent_of(path: &RepoPath) -> Self {
        match path.as_str().rsplit_once('/') {
            Some((dir, _)) => Self(dir.to_owned()),
            None => Self::root(),
        }
    }

    /// `self/name`, with `name` a single or multi-segment relative path.
    pub fn join(&self, name: &str) -> Result<RepoPath, CoreError> {
        if self.is_root() {
            RepoPath::new(name)
        } else {
            RepoPath::new(format!("{}/{}", self.0, name))
        }
    }

    /// True when `path` is this directory or inside it.
    pub fn contains(&self, path: &str) -> bool {
        if self.is_root() {
            return true;
        }
        path == self.0
            || path
                .strip_prefix(&self.0)
                .is_some_and(|r| r.starts_with('/'))
    }

    /// Number of path segments (0 for the root).
    pub fn depth(&self) -> usize {
        if self.is_root() {
            0
        } else {
            self.0.matches('/').count() + 1
        }
    }

    /// The last segment, or `""` for the root.
    pub fn basename(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }
}

impl From<RepoPath> for RepoDir {
    fn from(path: RepoPath) -> Self {
        Self(path.as_str().to_owned())
    }
}

impl fmt::Display for RepoDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RepoDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepoDir({:?})", self.0)
    }
}

impl<'de> Deserialize<'de> for RepoDir {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_nested() {
        let root = RepoDir::root();
        assert!(root.is_root());
        assert_eq!(root.join("a.json").unwrap().as_str(), "a.json");
        let dir = RepoDir::new("apps/api").unwrap();
        assert_eq!(
            dir.join("package.json").unwrap().as_str(),
            "apps/api/package.json"
        );
        assert!(dir.contains("apps/api/src/main.ts"));
        assert!(!dir.contains("apps/api2/src/main.ts"));
        assert_eq!(dir.depth(), 2);
        assert_eq!(dir.basename(), "api");
        assert!(RepoDir::new("../x").is_err());
        let p = RepoPath::new("apps/api/package.json").unwrap();
        assert_eq!(RepoDir::parent_of(&p), dir);
        let top = RepoPath::new("package.json").unwrap();
        assert!(RepoDir::parent_of(&top).is_root());
    }
}
