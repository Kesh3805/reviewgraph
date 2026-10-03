//! Repository discovery and git state through gix (INIT-001).
//!
//! Read-only. No subprocess is ever spawned and user/system git configuration is ignored, so the
//! result is identical on the CLI host and on a worker.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use review_core::ids::CommitSha;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{InitError, InitWarning};
use crate::remote_url::{redact, ProviderHint, RedactedUrl};

/// Entries examined by the dirty-status scan before it stops and sets `truncated`.
const STATUS_ENTRY_LIMIT: u64 = 200_000;
const SAMPLE_PATHS: usize = 20;
const WELL_KNOWN_BRANCHES: [&str; 4] = ["main", "master", "develop", "trunk"];

#[derive(Debug, Clone, Default)]
pub struct GitOpenOptions {
    /// Accept a directory that is not a git repository; `git` is then `None`.
    pub allow_non_git: bool,
    /// The provider's idea of the default branch, used when `origin/HEAD` is absent.
    pub provider_default_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DiscoveredRepo {
    /// Canonical worktree root.
    pub root: PathBuf,
    pub git: Option<GitState>,
    pub warnings: Vec<InitWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HeadState {
    Commit {
        sha: CommitSha,
        branch: Option<String>,
    },
    Detached {
        sha: CommitSha,
    },
    Unborn {
        branch: Option<String>,
    },
}

impl HeadState {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Commit { .. } => "commit",
            Self::Detached { .. } => "detached",
            Self::Unborn { .. } => "unborn",
        }
    }

    pub fn sha(&self) -> Option<&CommitSha> {
        match self {
            Self::Commit { sha, .. } | Self::Detached { sha } => Some(sha),
            Self::Unborn { .. } => None,
        }
    }

    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Commit { branch, .. } | Self::Unborn { branch } => branch.as_deref(),
            Self::Detached { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefaultBranchSource {
    OriginHead,
    Provider,
    WellKnownName,
    CurrentBranch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemoteFact {
    pub name: String,
    pub url: RedactedUrl,
    pub provider: ProviderHint,
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DirtyState {
    pub is_dirty: bool,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    /// At most 20 paths, sorted.
    pub sample_paths: Vec<RepoPath>,
    /// The scan stopped at the entry limit; the counts are lower bounds.
    pub truncated: bool,
    /// Every changed path (staged, unstaged, untracked), sorted. Runtime-only: used by the
    /// fingerprint, never serialized.
    #[serde(skip)]
    pub all_paths: Vec<RepoPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitState {
    pub head: HeadState,
    /// Sorted by name.
    pub remotes: Vec<RemoteFact>,
    pub default_branch: Option<String>,
    pub default_branch_source: DefaultBranchSource,
    pub dirty: DirtyState,
    pub is_shallow: bool,
    pub is_linked_worktree: bool,
    /// Paths from `.gitmodules`, sorted. Never recursed into.
    pub submodules: Vec<RepoPath>,
    /// `.gitattributes` patterns that carry `filter=lfs`, in file order.
    pub lfs_patterns: Vec<String>,
}

/// Finds the repository containing `path` and reports its git state.
pub fn discover(path: &Path, opts: &GitOpenOptions) -> Result<DiscoveredRepo, InitError> {
    let span = tracing::info_span!(
        "init.git_discover",
        git.head_kind = tracing::field::Empty,
        git.is_shallow = tracing::field::Empty,
        git.dirty = tracing::field::Empty,
        git.remote_count = tracing::field::Empty,
        git.default_branch_source = tracing::field::Empty,
    );
    let _guard = span.enter();

    let canonical = std::fs::canonicalize(path).map_err(|e| InitError::io(path, e))?;
    let repo = match open_isolated(&canonical) {
        Ok(repo) => repo,
        Err(OpenFailure::NotFound) => {
            if opts.allow_non_git {
                return Ok(DiscoveredRepo {
                    root: canonical,
                    git: None,
                    warnings: vec![InitWarning::new(
                        "not_a_git_repository",
                        None,
                        "directory is not inside a git repository",
                    )],
                });
            }
            return Err(InitError::NotAGitRepository(canonical));
        }
        Err(OpenFailure::Other(e)) => return Err(e),
    };
    if repo.is_bare() {
        return Err(InitError::BareRepository(repo.git_dir().to_path_buf()));
    }
    let Some(workdir) = repo.workdir() else {
        return Err(InitError::BareRepository(repo.git_dir().to_path_buf()));
    };
    let root = std::fs::canonicalize(workdir).map_err(|e| InitError::io(workdir, e))?;

    let mut warnings = Vec::new();
    let head = read_head(&repo, &mut warnings)?;
    let remotes = read_remotes(&repo, &mut warnings);
    let (default_branch, default_branch_source) =
        default_branch(&repo, &head, opts.provider_default_branch.as_deref());
    let dirty = dirty_state(&repo, &mut warnings);
    let is_linked_worktree = matches!(repo.kind(), gix::repository::Kind::LinkedWorkTree);
    let state = GitState {
        is_shallow: repo.is_shallow(),
        is_linked_worktree,
        submodules: read_submodules(&root, &mut warnings),
        lfs_patterns: read_lfs_patterns(&root),
        head,
        remotes,
        default_branch,
        default_branch_source,
        dirty,
    };
    span.record("git.head_kind", state.head.kind());
    span.record("git.is_shallow", state.is_shallow);
    span.record("git.dirty", state.dirty.is_dirty);
    span.record("git.remote_count", state.remotes.len());
    span.record(
        "git.default_branch_source",
        format!("{:?}", state.default_branch_source),
    );
    Ok(DiscoveredRepo {
        root,
        git: Some(state),
        warnings,
    })
}

enum OpenFailure {
    NotFound,
    Other(InitError),
}

fn open_isolated(path: &Path) -> Result<gix::Repository, OpenFailure> {
    let opts = gix::open::Options::isolated();
    let trust = gix::sec::trust::Mapping {
        full: opts.clone(),
        reduced: opts,
    };
    match gix::ThreadSafeRepository::discover_opts(path, Default::default(), trust) {
        Ok(repo) => Ok(repo.to_thread_local()),
        Err(e) => {
            let text = e.to_string();
            let lower = text.to_ascii_lowercase();
            if lower.contains("could not find a git repository")
                || lower.contains("not a git repository")
                || lower.contains("is not inside a git")
                || lower.contains("no git repository")
            {
                Err(OpenFailure::NotFound)
            } else {
                Err(OpenFailure::Other(InitError::git("discover", text)))
            }
        }
    }
}

fn read_head(
    repo: &gix::Repository,
    warnings: &mut Vec<InitWarning>,
) -> Result<HeadState, InitError> {
    let head = repo.head().map_err(|e| InitError::git("head", e))?;
    let branch = head
        .referent_name()
        .map(|n| n.shorten().to_string())
        .filter(|_| !head.is_detached());
    if head.is_unborn() {
        return Ok(HeadState::Unborn { branch });
    }
    let id = match head.id() {
        Some(id) => id.to_string(),
        None => {
            warnings.push(InitWarning::new(
                "head_unreadable",
                None,
                "HEAD does not resolve to a commit",
            ));
            return Ok(HeadState::Unborn { branch });
        }
    };
    let sha: CommitSha = id.parse()?;
    if head.is_detached() {
        Ok(HeadState::Detached { sha })
    } else {
        Ok(HeadState::Commit { sha, branch })
    }
}

fn read_remotes(repo: &gix::Repository, warnings: &mut Vec<InitWarning>) -> Vec<RemoteFact> {
    let mut out = Vec::new();
    let names: BTreeSet<String> = repo.remote_names().iter().map(|n| n.to_string()).collect();
    for name in names {
        let remote = match repo.find_remote(name.as_str()) {
            Ok(remote) => remote,
            Err(_) => {
                warnings.push(InitWarning::new(
                    "remote_unreadable",
                    None,
                    format!("remote `{name}` could not be read"),
                ));
                continue;
            }
        };
        let Some(url) = remote.url(gix::remote::Direction::Fetch) else {
            warnings.push(InitWarning::new(
                "remote_unreadable",
                None,
                format!("remote `{name}` has no fetch url"),
            ));
            continue;
        };
        let raw = url.to_bstring().to_string();
        let redacted = redact(&raw);
        out.push(RemoteFact {
            name,
            url: redacted.url,
            provider: redacted.provider,
            slug: redacted.slug,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn ref_exists(repo: &gix::Repository, name: &str) -> bool {
    matches!(repo.try_find_reference(name), Ok(Some(_)))
}

fn default_branch(
    repo: &gix::Repository,
    head: &HeadState,
    provider: Option<&str>,
) -> (Option<String>, DefaultBranchSource) {
    // 1. refs/remotes/origin/HEAD symbolic target.
    if let Ok(Some(reference)) = repo.try_find_reference("refs/remotes/origin/HEAD") {
        if let gix::refs::TargetRef::Symbolic(target) = reference.target() {
            let full = target.as_bstr().to_string();
            if let Some(branch) = full.strip_prefix("refs/remotes/origin/") {
                return (Some(branch.to_owned()), DefaultBranchSource::OriginHead);
            }
        }
    }
    // 2. Provider-supplied value.
    if let Some(branch) = provider.filter(|b| !b.is_empty()) {
        return (Some(branch.to_owned()), DefaultBranchSource::Provider);
    }
    // 3. Well-known names, local first, then origin.
    for prefix in ["refs/heads/", "refs/remotes/origin/"] {
        for name in WELL_KNOWN_BRANCHES {
            if ref_exists(repo, &format!("{prefix}{name}")) {
                return (Some(name.to_owned()), DefaultBranchSource::WellKnownName);
            }
        }
    }
    // 4. The current branch.
    if let Some(branch) = head.branch() {
        return (Some(branch.to_owned()), DefaultBranchSource::CurrentBranch);
    }
    (None, DefaultBranchSource::Unknown)
}

struct DirtyCollector {
    state: DirtyState,
    samples: BTreeSet<RepoPath>,
    all: BTreeSet<RepoPath>,
    entries: u64,
}

impl DirtyCollector {
    fn note(&mut self, path: &gix::bstr::BStr, warnings: &mut Vec<InitWarning>) {
        self.entries += 1;
        let Ok(text) = std::str::from_utf8(path) else {
            warnings.push(InitWarning::new(
                "non_utf8_path",
                None,
                "a changed path is not valid UTF-8 and was skipped",
            ));
            return;
        };
        if let Ok(rp) = RepoPath::new(text) {
            self.all.insert(rp.clone());
            self.samples.insert(rp);
            if self.samples.len() > SAMPLE_PATHS {
                self.samples.pop_last();
            }
        }
    }
}

fn dirty_state(repo: &gix::Repository, warnings: &mut Vec<InitWarning>) -> DirtyState {
    let mut collector = DirtyCollector {
        state: DirtyState::default(),
        samples: BTreeSet::new(),
        all: BTreeSet::new(),
        entries: 0,
    };
    let platform = match repo.status(gix::progress::Discard) {
        Ok(p) => p.untracked_files(gix::status::UntrackedFiles::Files),
        Err(_) => {
            warnings.push(InitWarning::new(
                "status_unavailable",
                None,
                "working tree status could not be computed",
            ));
            return collector.state;
        }
    };
    let iter = match platform.into_iter(Vec::<gix::bstr::BString>::new()) {
        Ok(iter) => iter,
        Err(_) => {
            warnings.push(InitWarning::new(
                "status_unavailable",
                None,
                "working tree status could not be computed",
            ));
            return collector.state;
        }
    };
    for item in iter {
        let Ok(item) = item else { continue };
        match item {
            gix::status::Item::TreeIndex(change) => {
                let (location, ..) = change.fields();
                if is_review_state(location) {
                    continue;
                }
                collector.state.staged += 1;
                collector.note(location, warnings);
            }
            gix::status::Item::IndexWorktree(item) => match item {
                gix::status::index_worktree::Item::Modification {
                    rela_path, status, ..
                } => {
                    use gix::status::plumbing::index_as_worktree::EntryStatus;
                    if matches!(
                        status,
                        EntryStatus::Change(_) | EntryStatus::Conflict { .. }
                    ) {
                        if is_review_state(rela_path.as_ref()) {
                            continue;
                        }
                        collector.state.unstaged += 1;
                        collector.note(rela_path.as_ref(), warnings);
                    }
                }
                gix::status::index_worktree::Item::DirectoryContents { entry, .. } => {
                    if entry.status == gix::dir::entry::Status::Untracked
                        && !is_review_state(entry.rela_path.as_ref())
                    {
                        collector.state.untracked += 1;
                        collector.note(entry.rela_path.as_ref(), warnings);
                    }
                }
                gix::status::index_worktree::Item::Rewrite {
                    dirwalk_entry,
                    copy,
                    ..
                } => {
                    if !copy && !is_review_state(dirwalk_entry.rela_path.as_ref()) {
                        collector.state.unstaged += 1;
                        collector.note(dirwalk_entry.rela_path.as_ref(), warnings);
                    }
                }
            },
        }
        if collector.entries >= STATUS_ENTRY_LIMIT {
            collector.state.truncated = true;
            break;
        }
    }
    let mut state = collector.state;
    state.sample_paths = collector.samples.into_iter().collect();
    state.all_paths = collector.all.into_iter().collect();
    state.is_dirty = state.staged + state.unstaged + state.untracked > 0;
    state
}

fn read_submodules(root: &Path, warnings: &mut Vec<InitWarning>) -> Vec<RepoPath> {
    let Ok(text) = std::fs::read_to_string(root.join(".gitmodules")) else {
        return Vec::new();
    };
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "path" {
            continue;
        }
        let value = value.trim().trim_matches('"');
        match RepoPath::new(value) {
            Ok(rp) => {
                out.insert(rp);
            }
            Err(_) => warnings.push(InitWarning::new(
                "submodule_path_invalid",
                None,
                "a .gitmodules path is not a valid repository path",
            )),
        }
    }
    out.into_iter().collect()
}

fn read_lfs_patterns(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(".gitattributes")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(pattern) = parts.next() else {
            continue;
        };
        if parts.any(|attr| attr == "filter=lfs") {
            out.push(pattern.to_owned());
        }
    }
    out
}

/// The subset of `candidates` that is tracked in the git index of the repository at `root`.
/// Returns an empty set when `root` is not a repository or the index cannot be read.
pub fn tracked_among(root: &Path, candidates: &[RepoPath]) -> BTreeSet<RepoPath> {
    use gix::bstr::ByteSlice;
    let Ok(repo) = open_isolated(root) else {
        return BTreeSet::new();
    };
    let Ok(index) = repo.index_or_empty() else {
        return BTreeSet::new();
    };
    candidates
        .iter()
        .filter(|path| {
            index
                .entry_by_path(path.as_str().as_bytes().as_bstr())
                .is_some()
        })
        .cloned()
        .collect()
}

/// `.review/` is ReviewGraph's own state directory: its (committable) config and ignore file
/// must not make a worktree look dirty.
fn is_review_state(path: &gix::bstr::BStr) -> bool {
    path == ".review" || path.starts_with(b".review/")
}
