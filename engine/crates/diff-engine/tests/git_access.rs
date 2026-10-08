#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::str::FromStr;
use std::sync::Mutex;

use diff_engine::git::{
    object_reads_total, GitError, GitRepo, MergeBase, ReadLimits, TreeEntryKind,
};
use diff_engine::testkit::{FileSpec, RepoBuilder};
use review_core::ids::CommitSha;
use review_core::location::RepoPath;
use review_core::{Classify, ErrorClass};

fn mk_sha(hex: &str) -> CommitSha {
    CommitSha::from_str(hex).unwrap()
}

fn unknown_sha() -> CommitSha {
    mk_sha(&"0".repeat(40))
}

fn unknown_oid() -> gix::ObjectId {
    gix::ObjectId::from_hex(&[b'0'; 40]).unwrap()
}

fn path(p: &str) -> RepoPath {
    RepoPath::new(p).unwrap()
}

#[test]
fn opens_bare_and_worktree_repositories() {
    let mut bare = RepoBuilder::bare().unwrap();
    let bare_sha = bare
        .commit("root", &[], &[FileSpec::file("a.txt", "hello\n")])
        .unwrap();
    let repo = bare.open().unwrap();
    let resolved = repo.resolve_commit(&bare_sha).unwrap();
    assert_eq!(resolved.to_string(), bare_sha.as_str());

    let mirror = GitRepo::open_mirror(bare.path(), ReadLimits::default()).unwrap();
    assert!(mirror.commit_exists(&bare_sha).unwrap());

    let mut worktree = RepoBuilder::worktree().unwrap();
    let work_sha = worktree
        .commit("root", &[], &[FileSpec::file("b.txt", "work\n")])
        .unwrap();
    let repo = worktree.open().unwrap();
    assert!(repo.commit_exists(&work_sha).unwrap());
}

#[test]
fn rejects_paths_that_are_not_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let err = GitRepo::open(dir.path(), ReadLimits::default()).unwrap_err();
    assert!(matches!(err, GitError::NotARepository(_)), "got {err:?}");
    assert_eq!(err.class(), ErrorClass::InvalidInput);

    let missing = dir.path().join("does-not-exist");
    let err = GitRepo::open(&missing, ReadLimits::default()).unwrap_err();
    assert!(matches!(err, GitError::NotARepository(_)), "got {err:?}");
}

#[test]
fn resolves_full_sha_and_rejects_short_or_ref() {
    assert!(CommitSha::from_str("abc1234").is_err());
    assert!(CommitSha::from_str("refs/heads/main").is_err());
    assert!(CommitSha::from_str("HEAD").is_err());

    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit("root", &[], &[FileSpec::file("a.txt", "hello\n")])
        .unwrap();
    let repo = fixture.open().unwrap();

    assert_eq!(repo.resolve_commit(&sha).unwrap().to_string(), sha.as_str());

    let err = repo.resolve_commit(&unknown_sha()).unwrap_err();
    assert!(matches!(err, GitError::ObjectNotFound(_)), "got {err:?}");
    assert_eq!(err.class(), ErrorClass::NotFound);

    let entries = repo.list_tree(&sha, None).unwrap();
    let blob_sha = mk_sha(entries[0].oid.to_string().as_str());
    let err = repo.resolve_commit(&blob_sha).unwrap_err();
    assert!(matches!(err, GitError::NotACommit { .. }), "got {err:?}");
    assert_eq!(err.class(), ErrorClass::InvalidInput);
}

#[test]
fn commit_exists_reports_presence() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit("root", &[], &[FileSpec::file("a.txt", "hello\n")])
        .unwrap();
    let repo = fixture.open().unwrap();

    assert!(repo.commit_exists(&sha).unwrap());
    assert!(!repo.commit_exists(&unknown_sha()).unwrap());
}

#[test]
fn tree_of_and_read_blob_by_oid_work() {
    let content = "export const x = 1;\n";
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit(
            "root",
            &[],
            &[FileSpec::file("src/auth/auth.service.ts", content)],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    let tree_oid = repo.tree_of(&sha).unwrap();
    assert_ne!(tree_oid, unknown_oid());

    let entries = repo.list_tree(&sha, None).unwrap();
    let blob = repo.read_blob_by_oid(&entries[0].oid).unwrap();
    assert_eq!(blob.data, content.as_bytes());
    assert_eq!(blob.size, content.len() as u64);

    let err = repo.read_blob_by_oid(&unknown_oid()).unwrap_err();
    assert!(matches!(err, GitError::ObjectNotFound(_)), "got {err:?}");

    let err = repo.read_blob_by_oid(&tree_oid).unwrap_err();
    assert!(matches!(err, GitError::NotABlob { .. }), "got {err:?}");
    assert_eq!(err.class(), ErrorClass::InvalidInput);
}

#[test]
fn merge_base_linear_and_branching() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let a = fixture
        .commit("a", &[], &[FileSpec::file("f.txt", "1")])
        .unwrap();
    let b = fixture
        .commit(
            "b",
            std::slice::from_ref(&a),
            &[FileSpec::file("f.txt", "2")],
        )
        .unwrap();
    let c = fixture
        .commit(
            "c",
            std::slice::from_ref(&b),
            &[FileSpec::file("f.txt", "3")],
        )
        .unwrap();
    let side = fixture
        .commit(
            "side",
            std::slice::from_ref(&a),
            &[FileSpec::file("g.txt", "4")],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    assert_eq!(
        repo.merge_base(&c, &b).unwrap(),
        MergeBase::Found {
            sha: b.clone(),
            candidates: 1
        }
    );
    assert_eq!(
        repo.merge_base(&c, &a).unwrap(),
        MergeBase::Found {
            sha: a.clone(),
            candidates: 1
        }
    );
    assert_eq!(
        repo.merge_base(&a, &c).unwrap(),
        MergeBase::Found {
            sha: a.clone(),
            candidates: 1
        }
    );
    assert_eq!(
        repo.merge_base(&c, &side).unwrap(),
        MergeBase::Found {
            sha: a.clone(),
            candidates: 1
        }
    );
}

#[test]
fn unrelated_histories_returns_none() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let a = fixture
        .commit("a", &[], &[FileSpec::file("f.txt", "1")])
        .unwrap();
    let b = fixture
        .commit("b", &[], &[FileSpec::file("g.txt", "2")])
        .unwrap();
    let repo = fixture.open().unwrap();

    assert_eq!(repo.merge_base(&a, &b).unwrap(), MergeBase::None);
}

#[test]
fn merge_base_criss_cross_is_deterministic() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let root = fixture
        .commit("root", &[], &[FileSpec::file("f.txt", "0")])
        .unwrap();
    let left = fixture
        .commit(
            "left",
            std::slice::from_ref(&root),
            &[FileSpec::file("f.txt", "l")],
        )
        .unwrap();
    let right = fixture
        .commit(
            "right",
            std::slice::from_ref(&root),
            &[FileSpec::file("f.txt", "r")],
        )
        .unwrap();
    let merge1 = fixture
        .commit(
            "merge1",
            &[left.clone(), right.clone()],
            &[FileSpec::file("f.txt", "m1")],
        )
        .unwrap();
    let merge2 = fixture
        .commit(
            "merge2",
            &[left.clone(), right.clone()],
            &[FileSpec::file("f.txt", "m2")],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    let first = repo.merge_base(&merge1, &merge2).unwrap();
    let MergeBase::Found {
        sha: base,
        candidates,
    } = &first
    else {
        panic!("criss-cross commits must have merge bases");
    };
    assert_eq!(*candidates, 2);
    assert!(base == &left || base == &right, "unexpected base {base}");
    for _ in 0..10 {
        assert_eq!(repo.merge_base(&merge1, &merge2).unwrap(), first);
    }
}

#[test]
fn read_blob_at_commit_matches_content() {
    let content = "export const authorize = () => true;\n";
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit(
            "root",
            &[],
            &[
                FileSpec::file("src/auth/auth.service.ts", content),
                FileSpec::file("top.txt", "top\n"),
            ],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    let blob = repo
        .read_blob(&sha, &path("src/auth/auth.service.ts"))
        .unwrap()
        .unwrap();
    assert_eq!(blob.data, content.as_bytes());
    assert_eq!(blob.size, content.len() as u64);
}

#[test]
fn missing_path_returns_none() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit(
            "root",
            &[],
            &[FileSpec::file("src/auth/auth.service.ts", "x\n")],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    assert!(repo
        .read_blob(&sha, &path("missing.txt"))
        .unwrap()
        .is_none());
    assert!(repo
        .read_blob(&sha, &path("src/auth/missing.ts"))
        .unwrap()
        .is_none());
}

#[test]
fn read_blob_rejects_directories_and_symlinks() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit(
            "root",
            &[],
            &[
                FileSpec::file("src/a.ts", "1"),
                FileSpec::symlink("link.ts", "src/a.ts"),
            ],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    let err = repo.read_blob(&sha, &path("src")).unwrap_err();
    assert!(matches!(err, GitError::NotABlob { .. }), "got {err:?}");
    let err = repo.read_blob(&sha, &path("link.ts")).unwrap_err();
    assert!(matches!(err, GitError::NotABlob { .. }), "got {err:?}");
}

#[test]
fn oversize_blob_rejected_without_inflating() {
    let content = "0123456789";
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit("root", &[], &[FileSpec::file("big.txt", content)])
        .unwrap();

    let limits = ReadLimits {
        max_blob_bytes: 5,
        ..ReadLimits::default()
    };
    let repo = fixture.open_with(limits).unwrap();
    let err = repo.read_blob(&sha, &path("big.txt")).unwrap_err();
    match err {
        GitError::BlobTooLarge { size, limit, .. } => {
            assert_eq!(size, 10);
            assert_eq!(limit, 5);
        }
        other => panic!("expected BlobTooLarge, got {other:?}"),
    }
}

#[test]
fn blob_header_reports_size_without_inflating() {
    let content = "0123456789";
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit("root", &[], &[FileSpec::file("big.txt", content)])
        .unwrap();
    let repo = fixture.open().unwrap();

    let entries = repo.list_tree(&sha, None).unwrap();
    assert_eq!(repo.blob_header(&entries[0].oid).unwrap().size, 10);
    assert_eq!(repo.blob_size(&entries[0].oid).unwrap(), 10);

    let blob = repo.read_blob(&sha, &path("big.txt")).unwrap().unwrap();
    assert_eq!(blob.data, content.as_bytes());
}

#[test]
fn list_tree_returns_sorted_entries_under_prefix() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit(
            "root",
            &[],
            &[
                FileSpec::file("src/b.ts", "b"),
                FileSpec::file("src/a.ts", "a"),
                FileSpec::file("README.md", "r"),
                FileSpec::symlink("link.md", "README.md"),
                FileSpec::executable("scripts/run.sh", "#!/bin/sh\n"),
            ],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    let all = repo.list_tree(&sha, None).unwrap();
    let paths: Vec<&str> = all.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "README.md",
            "link.md",
            "scripts/run.sh",
            "src/a.ts",
            "src/b.ts"
        ]
    );
    let kinds: Vec<TreeEntryKind> = all.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            TreeEntryKind::Blob,
            TreeEntryKind::Link,
            TreeEntryKind::Blob,
            TreeEntryKind::Blob,
            TreeEntryKind::Blob,
        ]
    );

    let under_src = repo.list_tree(&sha, Some(&path("src"))).unwrap();
    let src_paths: Vec<&str> = under_src.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(src_paths, ["src/a.ts", "src/b.ts"]);

    assert!(repo
        .list_tree(&sha, Some(&path("missing")))
        .unwrap()
        .is_empty());
    assert!(repo
        .list_tree(&sha, Some(&path("README.md")))
        .unwrap()
        .is_empty());
}

#[test]
fn list_tree_enforces_entry_limit() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit(
            "root",
            &[],
            &[FileSpec::file("a.txt", "a"), FileSpec::file("b.txt", "b")],
        )
        .unwrap();
    let limits = ReadLimits {
        max_tree_entries: 1,
        ..ReadLimits::default()
    };
    let repo = fixture.open_with(limits).unwrap();
    let err = repo.list_tree(&sha, None).unwrap_err();
    match err {
        GitError::TreeTooLarge { limit } => assert_eq!(limit, 1),
        other => panic!("expected TreeTooLarge, got {other:?}"),
    }
    assert_eq!(err.class(), ErrorClass::InvalidInput);
}

#[test]
fn sha256_repo_supported() {
    let mut fixture = RepoBuilder::sha256_bare().unwrap();
    let sha = fixture
        .commit("root", &[], &[FileSpec::file("a.txt", "hello\n")])
        .unwrap();
    assert_eq!(sha.as_str().len(), 64);

    let repo = fixture.open().unwrap();
    assert!(repo.commit_exists(&sha).unwrap());
    let blob = repo.read_blob(&sha, &path("a.txt")).unwrap().unwrap();
    assert_eq!(blob.data, b"hello\n");
}

#[test]
fn shallow_boundary_reported() {
    let mut fixture = RepoBuilder::worktree().unwrap();
    let a = fixture
        .commit("a", &[], &[FileSpec::file("f.txt", "1")])
        .unwrap();
    let b = fixture
        .commit(
            "b",
            std::slice::from_ref(&a),
            &[FileSpec::file("f.txt", "2")],
        )
        .unwrap();
    let c = fixture
        .commit(
            "c",
            std::slice::from_ref(&b),
            &[FileSpec::file("f.txt", "3")],
        )
        .unwrap();

    let hex = a.as_str();
    let loose = fixture
        .git_dir()
        .join("objects")
        .join(&hex[..2])
        .join(&hex[2..]);
    assert!(loose.exists(), "expected loose object at {loose:?}");
    std::fs::remove_file(&loose).unwrap();

    let repo = fixture.open().unwrap();
    let err = repo.resolve_commit(&a).unwrap_err();
    assert!(matches!(err, GitError::ObjectNotFound(_)), "got {err:?}");
    assert_eq!(err.class(), ErrorClass::NotFound);

    std::fs::write(fixture.git_dir().join("shallow"), format!("{hex}\n")).unwrap();
    let err = repo.resolve_commit(&a).unwrap_err();
    assert!(
        matches!(err, GitError::ShallowBoundary { .. }),
        "got {err:?}"
    );
    assert_eq!(err.class(), ErrorClass::Conflict);

    let err = repo.merge_base(&c, &b).unwrap_err();
    assert!(
        matches!(err, GitError::ShallowBoundary { .. }),
        "got {err:?}"
    );

    let blob = repo.read_blob(&c, &path("f.txt")).unwrap().unwrap();
    assert_eq!(blob.data, b"3");
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, OsString)]) -> Self {
        let mut saved = Vec::with_capacity(vars.len());
        for (key, value) in vars {
            saved.push((*key, std::env::var_os(key)));
            std::env::set_var(key, value);
        }
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, previous) in &self.saved {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    match ENV_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[test]
fn isolated_from_global_config() {
    let _lock = lock_env();

    let content = "crlf-content\r\n";
    let mut ours = RepoBuilder::worktree().unwrap();
    let ours_sha = ours
        .commit("ours", &[], &[FileSpec::file("a.txt", content)])
        .unwrap();

    let mut theirs = RepoBuilder::worktree().unwrap();
    let _theirs_sha = theirs
        .commit("theirs", &[], &[FileSpec::file("other.txt", "x")])
        .unwrap();

    let fake_home = tempfile::tempdir().unwrap();
    std::fs::write(
        fake_home.path().join(".gitconfig"),
        "[core]\n\tautocrlf = true\n[include]\n\tpath = missing-include.conf\n\
         [url \"ssh://git@evil.example/\"]\n\tinsteadOf = https://github.com/\n",
    )
    .unwrap();

    let _guard = EnvGuard::set(&[
        ("HOME", fake_home.path().as_os_str().to_owned()),
        (
            "GIT_CONFIG_GLOBAL",
            fake_home.path().join(".gitconfig").as_os_str().to_owned(),
        ),
        ("GIT_DIR", theirs.git_dir().as_os_str().to_owned()),
        ("GIT_WORK_TREE", theirs.path().as_os_str().to_owned()),
        (
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            theirs.git_dir().join("objects").as_os_str().to_owned(),
        ),
    ]);

    let repo = ours.open().unwrap();
    let resolved = repo.resolve_commit(&ours_sha).unwrap();
    assert_eq!(resolved.to_string(), ours_sha.as_str());

    let blob = repo.read_blob(&ours_sha, &path("a.txt")).unwrap().unwrap();
    assert_eq!(blob.data, content.as_bytes());
}

#[test]
fn env_alternate_object_directories_are_ignored() {
    let _lock = lock_env();

    let mut other = RepoBuilder::bare().unwrap();
    let other_sha = other
        .commit("other", &[], &[FileSpec::file("x.txt", "x")])
        .unwrap();

    let mut ours = RepoBuilder::bare().unwrap();
    let ours_sha = ours
        .commit("ours", &[], &[FileSpec::file("y.txt", "y")])
        .unwrap();

    let objects = other.path().join("objects");
    let _guard = EnvGuard::set(&[(
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        objects.as_os_str().to_owned(),
    )]);

    let repo = ours.open().unwrap();
    assert!(repo.commit_exists(&ours_sha).unwrap());
    let err = repo.resolve_commit(&other_sha).unwrap_err();
    assert!(
        matches!(err, GitError::ObjectNotFound(_)),
        "alternate objects must be ignored, got {err:?}"
    );
}

#[test]
fn git_file_pointing_outside_root_is_rejected() {
    let mut target = RepoBuilder::worktree().unwrap();
    target
        .commit("target", &[], &[FileSpec::file("t.txt", "t")])
        .unwrap();

    let outer = tempfile::tempdir().unwrap();
    let gitdir = target.git_dir().to_string_lossy().replace('\\', "/");
    std::fs::write(outer.path().join(".git"), format!("gitdir: {gitdir}\n")).unwrap();

    let err = GitRepo::open(outer.path(), ReadLimits::default()).unwrap_err();
    assert!(matches!(err, GitError::SymlinkEscape(_)), "got {err:?}");
    assert_eq!(err.class(), ErrorClass::InvalidInput);
}

#[test]
fn concurrent_reads_from_32_threads() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit(
            "root",
            &[],
            &[
                FileSpec::file("a.txt", "aaaaaaaaaaaaaaaaaaaa"),
                FileSpec::file("dir/b.txt", "bbbbbbbbbbbbbbbbbbbb"),
            ],
        )
        .unwrap();
    let repo = fixture.open().unwrap();

    std::thread::scope(|scope| {
        for _ in 0..32 {
            let repo = &repo;
            let sha = &sha;
            scope.spawn(move || {
                for _ in 0..10 {
                    let blob = repo.read_blob(sha, &path("dir/b.txt")).unwrap().unwrap();
                    assert_eq!(blob.data, b"bbbbbbbbbbbbbbbbbbbb");
                    assert_eq!(repo.resolve_commit(sha).unwrap().to_string(), sha.as_str());
                    assert_eq!(repo.list_tree(sha, None).unwrap().len(), 2);
                }
            });
        }
    });
}

#[test]
fn handle_is_cloneable_debuggable_and_shareable() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GitRepo>();

    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit("root", &[], &[FileSpec::file("a.txt", "hello\n")])
        .unwrap();
    let repo = fixture.open().unwrap();
    let clone = repo.clone();

    assert!(format!("{repo:?}").contains("GitRepo"));
    assert_eq!(clone.limits(), ReadLimits::default());
    assert!(clone.commit_exists(&sha).unwrap());
}

#[test]
fn object_read_counter_increments() {
    let mut fixture = RepoBuilder::bare().unwrap();
    let sha = fixture
        .commit("root", &[], &[FileSpec::file("a.txt", "hello\n")])
        .unwrap();
    let repo = fixture.open().unwrap();

    let before = object_reads_total();
    repo.resolve_commit(&sha).unwrap();
    repo.read_blob(&sha, &path("a.txt")).unwrap().unwrap();
    assert!(object_reads_total() >= before + 2);
}

#[test]
fn bare_mirror_with_alternates_reads() {
    let outer = tempfile::tempdir().unwrap();
    let mirror = outer.path().join("mirror.git");
    std::fs::create_dir_all(mirror.join("refs/heads")).unwrap();
    std::fs::create_dir_all(mirror.join("objects")).unwrap();
    std::fs::write(mirror.join("HEAD"), "ref: refs/heads/main\n").unwrap();

    let mut src = RepoBuilder::bare().unwrap();
    let sha = src
        .commit("root", &[], &[FileSpec::file("a.txt", "hello\n")])
        .unwrap();
    std::fs::rename(src.git_dir().join("objects"), mirror.join("source-objects")).unwrap();

    std::fs::create_dir_all(mirror.join("objects/info")).unwrap();
    let alternates = mirror.join("source-objects");
    std::fs::write(
        mirror.join("objects/info/alternates"),
        format!("{}\n", alternates.display()),
    )
    .unwrap();

    let repo = GitRepo::open(&mirror, ReadLimits::default()).unwrap();
    assert!(repo.commit_exists(&sha).unwrap());
    let blob = repo.read_blob(&sha, &path("a.txt")).unwrap().unwrap();
    assert_eq!(blob.data, b"hello\n");

    let evil = outer.path().join("evil.git");
    std::fs::create_dir_all(evil.join("refs/heads")).unwrap();
    std::fs::create_dir_all(evil.join("objects/info")).unwrap();
    std::fs::write(evil.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        evil.join("objects/info/alternates"),
        format!("{}\n", alternates.display()),
    )
    .unwrap();
    let err = GitRepo::open(&evil, ReadLimits::default()).unwrap_err();
    assert!(matches!(err, GitError::SymlinkEscape(_)), "got {err:?}");
    assert_eq!(err.class(), ErrorClass::InvalidInput);
}

#[test]
fn module_never_spawns_subprocesses() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let needles = [
        "std::process::Command",
        "process::Command",
        "Command::new",
        "std::process::",
    ];
    let mut scanned = 0;
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                scanned += 1;
                for needle in needles {
                    assert!(!text.contains(needle), "{:?} must not use {needle}", path);
                }
            }
        }
    }
    assert!(scanned > 5, "scanned only {scanned} source files");
}
