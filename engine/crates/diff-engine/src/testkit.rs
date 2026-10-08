//! Deterministic git fixture construction for tests (DIFF-001, CHG-008).
//!
//! Fixture repositories are built with gix directly, so the engine under test never shells out
//! to `git`. Commits use a fixed identity and timestamp, which keeps object ids reproducible.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use review_core::ids::CommitSha;

use crate::git::{GitError, GitRepo, ReadLimits};

pub mod scenario;
pub mod units;

pub use scenario::{apply_patch, Scenario, ScenarioError};
pub use units::UnitBuilder;

/// Commit timestamp used by every fixture commit: `2026-01-01T00:00:00Z`.
pub const FIXTURE_EPOCH_SECONDS: i64 = 1_767_225_600;

/// File mode for a fixture file entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    /// Regular file (`100644`).
    Regular,
    /// Executable file (`100755`).
    Executable,
    /// Symbolic link (`120000`); `content` is the link target.
    Symlink,
    /// Submodule gitlink (`160000`); `content` is the target commit sha (40 hex characters).
    Gitlink,
}

/// One file to write into a fixture commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpec<'a> {
    /// Repository-relative path using `/` separators.
    pub path: &'a str,
    /// File content (or link target for [`FileMode::Symlink`]).
    pub content: &'a str,
    /// Entry mode.
    pub mode: FileMode,
}

impl<'a> FileSpec<'a> {
    /// A regular file.
    pub fn file(path: &'a str, content: &'a str) -> Self {
        Self {
            path,
            content,
            mode: FileMode::Regular,
        }
    }

    /// An executable file.
    pub fn executable(path: &'a str, content: &'a str) -> Self {
        Self {
            path,
            content,
            mode: FileMode::Executable,
        }
    }

    /// A symbolic link pointing at `target`.
    pub fn symlink(path: &'a str, target: &'a str) -> Self {
        Self {
            path,
            content: target,
            mode: FileMode::Symlink,
        }
    }

    /// A submodule gitlink entry pointing at `target_commit` (40 hex characters).
    pub fn gitlink(path: &'a str, target_commit: &'a str) -> Self {
        Self {
            path,
            content: target_commit,
            mode: FileMode::Gitlink,
        }
    }
}

/// Entry point for constructing throwaway fixture repositories.
#[derive(Debug, Clone, Copy)]
pub struct RepoBuilder;

impl RepoBuilder {
    /// An empty bare repository.
    pub fn bare() -> Result<FixtureRepo, GitError> {
        create(gix::create::Kind::Bare, None)
    }

    /// An empty repository with a worktree.
    pub fn worktree() -> Result<FixtureRepo, GitError> {
        create(gix::create::Kind::WithWorktree, None)
    }

    /// An empty bare repository using the SHA-256 object format.
    pub fn sha256_bare() -> Result<FixtureRepo, GitError> {
        create(gix::create::Kind::Bare, Some(gix::hash::Kind::Sha256))
    }
}

fn create(
    kind: gix::create::Kind,
    object_hash: Option<gix::hash::Kind>,
) -> Result<FixtureRepo, GitError> {
    let dir = tempfile::tempdir().map_err(|e| GitError::Io {
        path: PathBuf::from("<tempdir>"),
        source: e,
    })?;
    let gix = if let Some(object_hash) = object_hash {
        let tsr = gix::ThreadSafeRepository::init(
            dir.path(),
            kind,
            gix::create::Options {
                object_hash: Some(object_hash),
                ..Default::default()
            },
        )
        .map_err(|e| GitError::Gix(e.to_string()))?;
        tsr.to_thread_local()
    } else if matches!(kind, gix::create::Kind::Bare) {
        gix::init_bare(dir.path()).map_err(|e| GitError::Gix(e.to_string()))?
    } else {
        gix::init(dir.path()).map_err(|e| GitError::Gix(e.to_string()))?
    };
    Ok(FixtureRepo {
        dir,
        gix,
        head: None,
    })
}

/// A throwaway repository under the system temporary directory.
#[derive(Debug)]
pub struct FixtureRepo {
    dir: tempfile::TempDir,
    gix: gix::Repository,
    head: Option<CommitSha>,
}

impl FixtureRepo {
    /// Root directory of the repository.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The repository's git directory (`.git` for worktrees, the root for bare repos).
    pub fn git_dir(&self) -> PathBuf {
        self.gix.git_dir().to_path_buf()
    }

    /// Sha of the most recent [`FixtureRepo::commit`], if any.
    pub fn head(&self) -> Option<&CommitSha> {
        self.head.as_ref()
    }

    /// Open the fixture with default [`ReadLimits`].
    pub fn open(&self) -> Result<GitRepo, GitError> {
        GitRepo::open(self.path(), ReadLimits::default())
    }

    /// Open the fixture with custom [`ReadLimits`].
    pub fn open_with(&self, limits: ReadLimits) -> Result<GitRepo, GitError> {
        GitRepo::open(self.path(), limits)
    }

    /// Write the given files as a new commit with the fixed fixture identity and timestamp.
    pub fn commit(
        &mut self,
        message: &str,
        parents: &[CommitSha],
        files: &[FileSpec<'_>],
    ) -> Result<CommitSha, GitError> {
        let mut parent_ids = Vec::with_capacity(parents.len());
        for parent in parents {
            parent_ids.push(
                gix::ObjectId::from_hex(parent.as_str().as_bytes())
                    .map_err(|_| GitError::InvalidObjectId(parent.as_str().to_owned()))?,
            );
        }
        let tree = write_tree(&self.gix, files)?;
        let commit = RawCommit::new(&tree, &parent_ids, message);
        let id = self
            .gix
            .write_object(commit)
            .map_err(|e| GitError::Gix(e.to_string()))?
            .detach();
        let sha =
            CommitSha::from_str(&id.to_string()).map_err(|e| GitError::Corrupt(e.to_string()))?;
        self.head = Some(sha.clone());
        Ok(sha)
    }
}

enum Entry {
    File(gix::ObjectId, FileMode),
    Dir(BTreeMap<String, Entry>),
}

fn write_tree(gix: &gix::Repository, files: &[FileSpec<'_>]) -> Result<gix::ObjectId, GitError> {
    let mut root: BTreeMap<String, Entry> = BTreeMap::new();
    for file in files {
        insert(gix, &mut root, file)?;
    }
    write_dir(gix, &root)
}

fn insert(
    gix: &gix::Repository,
    map: &mut BTreeMap<String, Entry>,
    file: &FileSpec<'_>,
) -> Result<(), GitError> {
    let parts: Vec<&str> = file
        .path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return Err(GitError::Corrupt(format!(
            "empty fixture path: {:?}",
            file.path
        )));
    }
    insert_at(gix, map, &parts, file)
}

fn insert_at(
    gix: &gix::Repository,
    map: &mut BTreeMap<String, Entry>,
    parts: &[&str],
    file: &FileSpec<'_>,
) -> Result<(), GitError> {
    let Some((name, rest)) = parts.split_first() else {
        return Err(GitError::Corrupt(format!(
            "empty fixture path: {:?}",
            file.path
        )));
    };
    if rest.is_empty() {
        let oid = if file.mode == FileMode::Gitlink {
            gix::ObjectId::from_hex(file.content.as_bytes()).map_err(|_| {
                GitError::Corrupt(format!("invalid gitlink target sha: {:?}", file.content))
            })?
        } else {
            write_blob(gix, file.content)?
        };
        map.insert(name.to_string(), Entry::File(oid, file.mode));
        return Ok(());
    }
    let entry = map
        .entry(name.to_string())
        .or_insert_with(|| Entry::Dir(BTreeMap::new()));
    match entry {
        Entry::Dir(dir) => insert_at(gix, dir, rest, file),
        Entry::File(..) => Err(GitError::Corrupt(format!(
            "fixture path conflicts with a file: {:?}",
            file.path
        ))),
    }
}

fn write_blob(gix: &gix::Repository, content: &str) -> Result<gix::ObjectId, GitError> {
    gix::Repository::write_object(gix, RawBlob(content.as_bytes().to_vec()))
        .map(|id| id.detach())
        .map_err(|e| GitError::Gix(e.to_string()))
}

fn write_dir(
    gix: &gix::Repository,
    map: &BTreeMap<String, Entry>,
) -> Result<gix::ObjectId, GitError> {
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(map.len());
    for (name, entry) in map {
        let (mode, is_dir, oid) = match entry {
            Entry::File(oid, mode) => (mode_str(*mode), false, *oid),
            Entry::Dir(dir) => ("40000", true, write_dir(gix, dir)?),
        };
        let mut key = name.as_bytes().to_vec();
        if is_dir {
            key.push(b'/');
        }
        let mut encoded = Vec::with_capacity(name.len() + 26);
        encoded.extend_from_slice(mode.as_bytes());
        encoded.push(b' ');
        encoded.extend_from_slice(name.as_bytes());
        encoded.push(0);
        encoded.extend_from_slice(oid.as_bytes());
        entries.push((key, encoded));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut data = Vec::new();
    for (_, encoded) in entries {
        data.extend_from_slice(&encoded);
    }
    gix::Repository::write_object(gix, RawTree(data))
        .map(|id| id.detach())
        .map_err(|e| GitError::Gix(e.to_string()))
}

fn mode_str(mode: FileMode) -> &'static str {
    match mode {
        FileMode::Regular => "100644",
        FileMode::Executable => "100755",
        FileMode::Symlink => "120000",
        FileMode::Gitlink => "160000",
    }
}

/// Raw blob bytes awaiting object encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBlob(Vec<u8>);

impl gix::objs::WriteTo for RawBlob {
    fn write_to(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        out.write_all(&self.0)
    }

    fn kind(&self) -> gix::objs::Kind {
        gix::objs::Kind::Blob
    }

    fn size(&self) -> u64 {
        self.0.len() as u64
    }
}

/// Raw tree bytes awaiting object encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTree(Vec<u8>);

impl gix::objs::WriteTo for RawTree {
    fn write_to(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        out.write_all(&self.0)
    }

    fn kind(&self) -> gix::objs::Kind {
        gix::objs::Kind::Tree
    }

    fn size(&self) -> u64 {
        self.0.len() as u64
    }
}

/// Raw commit bytes awaiting object encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCommit(Vec<u8>);

impl RawCommit {
    fn new(tree: &gix::ObjectId, parents: &[gix::ObjectId], message: &str) -> Self {
        let mut bytes = format!("tree {tree}\n");
        for parent in parents {
            bytes.push_str(&format!("parent {parent}\n"));
        }
        bytes.push_str(&format!(
            "author fixture <fixture@example.com> {FIXTURE_EPOCH_SECONDS} +0000\n"
        ));
        bytes.push_str(&format!(
            "committer fixture <fixture@example.com> {FIXTURE_EPOCH_SECONDS} +0000\n"
        ));
        bytes.push('\n');
        bytes.push_str(message);
        if !message.ends_with('\n') {
            bytes.push('\n');
        }
        Self(bytes.into_bytes())
    }
}

impl gix::objs::WriteTo for RawCommit {
    fn write_to(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        out.write_all(&self.0)
    }

    fn kind(&self) -> gix::objs::Kind {
        gix::objs::Kind::Commit
    }

    fn size(&self) -> u64 {
        self.0.len() as u64
    }
}
