//! Deterministic repository file walk (INIT-002).
//!
//! One authoritative list of files: .gitignore, .reviewignore and configured globs applied,
//! vendored directories skipped, symlinks never followed, sensitive files never opened.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ignore::overrides::OverrideBuilder;
use ignore::{WalkBuilder, WalkState};
use review_core::location::RepoPath;
use serde::{Deserialize, Serialize};

use crate::error::{InitError, InitWarning};
use crate::read::{FileSource, OsFiles};
use crate::sensitive::is_sensitive;

const BUILTIN_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".review",
    ".hg",
    ".svn",
    "bower_components",
    ".pnpm-store",
];

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "tiff", "pdf", "zip", "gz", "tgz", "bz2",
    "xz", "7z", "rar", "jar", "war", "class", "so", "dylib", "dll", "exe", "bin", "o", "a", "wasm",
    "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov", "avi", "webm", "sqlite", "db",
    "pyc", "parquet",
];

const LFS_POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";
const SNIFF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Larger files are `TooLarge`: counted, never parsed. Default 1 MiB.
    pub max_analyze_bytes: u64,
    /// Larger files are never opened at all. Default 16 MiB.
    pub hard_max_bytes: u64,
    /// More entries than this is `InitError::TooManyFiles`. Default 500,000.
    pub max_files: u64,
    /// From `.review/config.yaml` `ignore:`.
    pub extra_ignore_globs: Vec<String>,
    /// Honour `.git/info/exclude`. Off by default for host/worker parity.
    pub respect_local_excludes: bool,
    /// Walker threads; `None` is `min(available_parallelism, 8)`.
    pub threads: Option<usize>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            max_analyze_bytes: 1024 * 1024,
            hard_max_bytes: 16 * 1024 * 1024,
            max_files: 500_000,
            extra_ignore_globs: Vec::new(),
            respect_local_excludes: false,
            threads: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileClass {
    Source,
    Binary,
    TooLarge,
    LfsPointer,
    Sensitive,
    Symlink,
}

impl FileClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Binary => "binary",
            Self::TooLarge => "too_large",
            Self::LfsPointer => "lfs_pointer",
            Self::Sensitive => "sensitive",
            Self::Symlink => "symlink",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreReason {
    Gitignore,
    Reviewignore,
    ConfigGlob,
    BuiltinDir,
    NodeModules,
    /// Unused: hidden files are walked.
    Hidden,
    NonUtf8Path,
    OutsideRoot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymlinkTarget {
    /// The link target inside the repository, if it stays inside.
    pub relative: Option<RepoPath>,
    pub escapes_root: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: RepoPath,
    pub size: u64,
    pub class: FileClass,
    pub symlink_target: Option<SymlinkTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInventory {
    /// Canonical root. Never serialized into `repository.json`.
    pub root: PathBuf,
    /// Sorted by path.
    pub entries: Vec<FileEntry>,
    /// Entries skipped by rules the walker can observe (built-in directories, bad paths).
    /// Files excluded by .gitignore/.reviewignore/config globs are not enumerated, so those
    /// reasons are not counted.
    pub ignored: BTreeMap<IgnoreReason, u64>,
    /// Sorted directories (relative), used by glob expansion.
    pub dirs: Vec<RepoPath>,
}

impl FileInventory {
    pub fn find(&self, path: &str) -> Option<&FileEntry> {
        self.entries
            .binary_search_by(|e| e.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.entries[i])
    }

    pub fn count_by_class(&self) -> BTreeMap<FileClass, u64> {
        let mut out = BTreeMap::new();
        for entry in &self.entries {
            *out.entry(entry.class).or_insert(0) += 1;
        }
        out
    }
}

/// Walks `root` with the real file system.
pub fn walk(
    root: &Path,
    opts: &WalkOptions,
) -> Result<(FileInventory, Vec<InitWarning>), InitError> {
    walk_with(root, opts, Arc::new(OsFiles))
}

struct Shared {
    root: PathBuf,
    opts: WalkOptions,
    source: Arc<dyn FileSource>,
    entries: Mutex<Vec<FileEntry>>,
    dirs: Mutex<Vec<RepoPath>>,
    warnings: Mutex<Vec<InitWarning>>,
    count: AtomicU64,
    too_many: AtomicBool,
    builtin_skipped: AtomicU64,
    node_modules_skipped: AtomicU64,
    non_utf8: AtomicU64,
    outside: AtomicU64,
}

impl Shared {
    fn warn(&self, warning: InitWarning) {
        if let Ok(mut guard) = self.warnings.lock() {
            guard.push(warning);
        }
    }
}

fn is_builtin_dir(entry: &ignore::DirEntry) -> Option<IgnoreReason> {
    if entry.depth() == 0 || !entry.file_type().is_some_and(|t| t.is_dir()) {
        return None;
    }
    let name = entry.file_name().to_str()?;
    if name == "node_modules" {
        return Some(IgnoreReason::NodeModules);
    }
    if BUILTIN_DIRS.contains(&name) {
        return Some(IgnoreReason::BuiltinDir);
    }
    if name == "cache" {
        let parent = entry.path().parent()?.file_name()?.to_str()?;
        if parent == ".yarn" {
            return Some(IgnoreReason::BuiltinDir);
        }
    }
    None
}

/// Like [`walk`], reading file prefixes through `source`.
pub fn walk_with(
    root: &Path,
    opts: &WalkOptions,
    source: Arc<dyn FileSource>,
) -> Result<(FileInventory, Vec<InitWarning>), InitError> {
    let span = tracing::info_span!(
        "init.walk",
        walk.entries = tracing::field::Empty,
        walk.ignored = tracing::field::Empty,
        walk.threads = tracing::field::Empty
    );
    let _guard = span.enter();

    let root = std::fs::canonicalize(root).map_err(|e| InitError::io(root, e))?;
    let threads = opts.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4)
    });
    let shared = Arc::new(Shared {
        root: root.clone(),
        opts: opts.clone(),
        source,
        entries: Mutex::new(Vec::new()),
        dirs: Mutex::new(Vec::new()),
        warnings: Mutex::new(Vec::new()),
        count: AtomicU64::new(0),
        too_many: AtomicBool::new(false),
        builtin_skipped: AtomicU64::new(0),
        node_modules_skipped: AtomicU64::new(0),
        non_utf8: AtomicU64::new(0),
        outside: AtomicU64::new(0),
    });

    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(opts.respect_local_excludes)
        .parents(false)
        .require_git(false)
        .ignore(false)
        .follow_links(false)
        .same_file_system(true)
        .threads(threads)
        .add_custom_ignore_filename(".reviewignore");
    if !opts.extra_ignore_globs.is_empty() {
        let mut overrides = OverrideBuilder::new(&root);
        for glob in &opts.extra_ignore_globs {
            if overrides.add(&format!("!{glob}")).is_err() {
                shared.warn(InitWarning::new(
                    "ignore_parse",
                    None,
                    format!("ignore glob `{glob}` is invalid and was skipped"),
                ));
            }
        }
        match overrides.build() {
            Ok(built) => {
                builder.overrides(built);
            }
            Err(_) => shared.warn(InitWarning::new(
                "ignore_parse",
                None,
                "configured ignore globs could not be compiled",
            )),
        }
    }
    let filter_state = Arc::clone(&shared);
    builder.filter_entry(move |entry| match is_builtin_dir(entry) {
        Some(IgnoreReason::NodeModules) => {
            filter_state
                .node_modules_skipped
                .fetch_add(1, Ordering::Relaxed);
            false
        }
        Some(_) => {
            filter_state.builtin_skipped.fetch_add(1, Ordering::Relaxed);
            false
        }
        None => true,
    });

    builder.build_parallel().run(|| {
        let shared = Arc::clone(&shared);
        Box::new(move |result| visit(&shared, result))
    });

    if shared.too_many.load(Ordering::Relaxed) {
        return Err(InitError::TooManyFiles {
            limit: opts.max_files,
        });
    }

    let mut entries = take(&shared.entries);
    entries.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    let mut dirs = take(&shared.dirs);
    dirs.sort_unstable();
    let mut warnings = take(&shared.warnings);

    // Case collisions: two paths that differ only by case.
    let mut seen: HashMap<String, &RepoPath> = HashMap::new();
    for entry in &entries {
        let key = entry.path.as_str().to_lowercase();
        if let Some(first) = seen.get(&key) {
            warnings.push(InitWarning::new(
                "case_collision",
                Some(entry.path.clone()),
                format!(
                    "`{}` and `{}` differ only by case",
                    first.as_str(),
                    entry.path.as_str()
                ),
            ));
        } else {
            seen.insert(key, &entry.path);
        }
    }
    warnings.sort();
    warnings.dedup();

    let mut ignored = BTreeMap::new();
    for (reason, counter) in [
        (IgnoreReason::BuiltinDir, &shared.builtin_skipped),
        (IgnoreReason::NodeModules, &shared.node_modules_skipped),
        (IgnoreReason::NonUtf8Path, &shared.non_utf8),
        (IgnoreReason::OutsideRoot, &shared.outside),
    ] {
        let n = counter.load(Ordering::Relaxed);
        if n > 0 {
            ignored.insert(reason, n);
        }
    }

    span.record("walk.entries", entries.len());
    span.record("walk.ignored", ignored.values().sum::<u64>());
    span.record("walk.threads", threads);
    Ok((
        FileInventory {
            root,
            entries,
            ignored,
            dirs,
        },
        warnings,
    ))
}

fn take<T>(m: &Mutex<Vec<T>>) -> Vec<T> {
    m.lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

fn visit(shared: &Shared, result: Result<ignore::DirEntry, ignore::Error>) -> WalkState {
    let entry = match result {
        Ok(entry) => entry,
        Err(error) => {
            let code = if matches!(error, ignore::Error::Partial(_)) {
                "ignore_parse"
            } else {
                "unreadable_entry"
            };
            shared.warn(InitWarning::new(
                code,
                None,
                "an entry or ignore rule could not be read and was skipped",
            ));
            return WalkState::Continue;
        }
    };
    if entry.depth() == 0 {
        return WalkState::Continue;
    }
    let path = entry.path();
    let Ok(relative) = path.strip_prefix(&shared.root) else {
        shared.outside.fetch_add(1, Ordering::Relaxed);
        return WalkState::Continue;
    };
    let Some(relative_text) = relative.to_str() else {
        shared.non_utf8.fetch_add(1, Ordering::Relaxed);
        shared.warn(InitWarning::new(
            "non_utf8_path",
            None,
            "a path is not valid UTF-8 and was skipped",
        ));
        return WalkState::Continue;
    };
    let relative_text = relative_text.replace('\\', "/");
    let Ok(repo_path) = RepoPath::new(relative_text) else {
        shared.outside.fetch_add(1, Ordering::Relaxed);
        return WalkState::Continue;
    };

    let is_symlink = entry.path_is_symlink();
    let file_type = entry.file_type();
    if !is_symlink && file_type.is_some_and(|t| t.is_dir()) {
        if let Ok(mut dirs) = shared.dirs.lock() {
            dirs.push(repo_path);
        }
        return WalkState::Continue;
    }

    let count = shared.count.fetch_add(1, Ordering::Relaxed) + 1;
    if count > shared.opts.max_files {
        shared.too_many.store(true, Ordering::Relaxed);
        return WalkState::Quit;
    }

    let file_entry = classify(shared, &repo_path, path, is_symlink);
    if let Ok(mut entries) = shared.entries.lock() {
        entries.push(file_entry);
    }
    WalkState::Continue
}

fn classify(shared: &Shared, repo_path: &RepoPath, absolute: &Path, is_symlink: bool) -> FileEntry {
    let size = std::fs::symlink_metadata(absolute)
        .map(|m| m.len())
        .unwrap_or(0);
    let make = |class, symlink_target| FileEntry {
        path: repo_path.clone(),
        size,
        class,
        symlink_target,
    };

    // 1. Symlinks: record the target, never follow it.
    if is_symlink {
        let target = symlink_target(&shared.root, absolute);
        if target.escapes_root {
            shared.warn(InitWarning::new(
                "symlink_escapes_root",
                Some(repo_path.clone()),
                "symlink target points outside the repository",
            ));
        }
        return make(FileClass::Symlink, Some(target));
    }
    // 2. Sensitive names: never opened.
    if is_sensitive(repo_path) {
        return make(FileClass::Sensitive, None);
    }
    // 3. Hard size limit: never opened.
    if size > shared.opts.hard_max_bytes {
        return make(FileClass::TooLarge, None);
    }
    // 4. Known binary extension.
    if let Some(ext) = repo_path.extension() {
        let ext = ext.to_ascii_lowercase();
        if BINARY_EXTENSIONS.contains(&ext.as_str()) {
            return make(FileClass::Binary, None);
        }
    }
    // 5. Content sniff: NUL byte or git-lfs pointer.
    match shared.source.read_prefix(absolute, SNIFF_BYTES) {
        Ok(prefix) => {
            if prefix.contains(&0) {
                return make(FileClass::Binary, None);
            }
            if prefix.starts_with(LFS_POINTER_PREFIX) {
                return make(FileClass::LfsPointer, None);
            }
        }
        Err(_) => {
            shared.warn(InitWarning::new(
                "unreadable_entry",
                Some(repo_path.clone()),
                "file could not be read",
            ));
        }
    }
    // 6. Analyze limit.
    if size > shared.opts.max_analyze_bytes {
        return make(FileClass::TooLarge, None);
    }
    make(FileClass::Source, None)
}

fn symlink_target(root: &Path, link: &Path) -> SymlinkTarget {
    let Ok(target) = std::fs::read_link(link) else {
        return SymlinkTarget {
            relative: None,
            escapes_root: false,
        };
    };
    let base = if target.is_absolute() {
        PathBuf::new()
    } else {
        link.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let mut resolved = base;
    for component in target.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }
    match resolved.strip_prefix(root) {
        Ok(rest) => {
            let text = rest.to_string_lossy().replace('\\', "/");
            SymlinkTarget {
                relative: RepoPath::new(text).ok(),
                escapes_root: false,
            }
        }
        Err(_) => SymlinkTarget {
            relative: None,
            escapes_root: true,
        },
    }
}
