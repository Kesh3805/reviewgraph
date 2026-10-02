#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use proptest::prelude::*;
use repository::read::{BoundedReader, FileSource, OsFiles, ReadError};
use repository::walk::{walk, walk_with, FileClass, FileInventory, WalkOptions};
use repository::{InitError, InitWarning};
use review_core::location::RepoPath;
use review_test_support::{edge_case_tree, write_file, CANARIES};

fn walked(tmp: &Path) -> (FileInventory, Vec<InitWarning>) {
    walk(tmp, &WalkOptions::default()).unwrap()
}

fn paths(inv: &FileInventory) -> BTreeSet<&str> {
    inv.entries.iter().map(|e| e.path.as_str()).collect()
}

fn class_of(inv: &FileInventory, path: &str) -> FileClass {
    inv.find(path)
        .unwrap_or_else(|| panic!("{path} not in inventory"))
        .class
}

/// Records every path handed to `read_prefix`.
#[derive(Debug, Default)]
struct CountingFiles {
    opened: Mutex<Vec<PathBuf>>,
}

impl FileSource for CountingFiles {
    fn read_prefix(&self, absolute: &Path, max: usize) -> io::Result<Vec<u8>> {
        self.opened.lock().unwrap().push(absolute.to_path_buf());
        OsFiles.read_prefix(absolute, max)
    }
}

#[test]
fn gitignore_nested_rules_respected() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    let p = paths(&inv);
    assert!(p.contains("src/app.ts"));
    assert!(!p.contains("src/ignored-by-git.ts"));
    assert!(!p.contains("debug.log"));
    assert!(!p.contains("build-cache/x.js"));
    assert!(!p.contains("packages/a/local.ts"), "nested .gitignore");
    assert!(p.contains("packages/a/index.ts"));
}

#[test]
fn reviewignore_applies_like_gitignore() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    let p = paths(&inv);
    assert!(!p.contains("docs-private/notes.md"));
    assert!(!p.contains("packages/b/scratch.ts"), "nested .reviewignore");
    assert!(p.contains("packages/b/index.ts"));
}

#[test]
fn config_ignore_globs_applied() {
    let tmp = edge_case_tree();
    let opts = WalkOptions {
        extra_ignore_globs: vec!["config-ignored/**".to_owned(), "**/*.pb.go".to_owned()],
        ..WalkOptions::default()
    };
    let (inv, warnings) = walk(tmp.path(), &opts).unwrap();
    let p = paths(&inv);
    assert!(!p.contains("config-ignored/skip.ts"));
    assert!(!p.contains("api.pb.go"));
    assert!(p.contains("src/app.ts"));
    assert!(warnings.iter().all(|w| w.code != "ignore_parse"));
}

#[test]
fn node_modules_always_skipped_even_if_not_gitignored() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    assert!(inv
        .entries
        .iter()
        .all(|e| !e.path.as_str().contains("node_modules")));
    assert!(inv.ignored.values().sum::<u64>() >= 2);
}

#[test]
fn hidden_dirs_like_github_are_walked() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    assert!(paths(&inv).contains(".github/workflows/ci.yml"));
    assert!(paths(&inv).contains(".gitignore"));
    assert!(inv.dirs.iter().any(|d| d.as_str() == ".github/workflows"));
}

#[test]
fn global_gitignore_not_applied() {
    let tmp = edge_case_tree();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".gitignore_global"), "app.ts\n").unwrap();
    std::fs::write(
        home.path().join(".gitconfig"),
        format!(
            "[core]\n\texcludesfile = {}\n",
            home.path().join(".gitignore_global").display()
        ),
    )
    .unwrap();
    let previous = std::env::var_os("HOME");
    std::env::set_var("HOME", home.path());
    let (inv, _) = walked(tmp.path());
    match previous {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    assert!(paths(&inv).contains("src/app.ts"));
}

#[test]
fn binary_by_extension_and_by_nul_sniff() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    assert_eq!(class_of(&inv, "logo.png"), FileClass::Binary);
    assert_eq!(class_of(&inv, "blob.dat"), FileClass::Binary);
    assert_eq!(class_of(&inv, "src/app.ts"), FileClass::Source);
}

#[test]
fn lfs_pointer_detected() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    assert_eq!(class_of(&inv, "model.weights"), FileClass::LfsPointer);
}

#[test]
fn too_large_counted_not_read() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    let big = inv.find("big.txt").unwrap();
    assert_eq!(big.class, FileClass::TooLarge);
    assert_eq!(big.size, 2 * 1024 * 1024);
    let reader = BoundedReader::new(tmp.path());
    assert!(matches!(
        reader.read_prefix(big, 10),
        Err(ReadError::Refused(FileClass::TooLarge))
    ));
}

#[test]
fn hard_max_file_never_opened() {
    let tmp = edge_case_tree();
    let counting = Arc::new(CountingFiles::default());
    let opts = WalkOptions {
        hard_max_bytes: 1024 * 1024,
        ..WalkOptions::default()
    };
    let (inv, _) = walk_with(tmp.path(), &opts, counting.clone()).unwrap();
    assert_eq!(class_of(&inv, "big.txt"), FileClass::TooLarge);
    let opened = counting.opened.lock().unwrap();
    assert!(!opened.is_empty(), "the counting double must be exercised");
    assert!(
        opened.iter().all(|p| !p.ends_with("big.txt")),
        "files above hard_max_bytes must never be opened"
    );
}

#[cfg(unix)]
#[test]
fn symlink_not_followed_and_escape_warned() {
    let tmp = edge_case_tree();
    let (inv, warnings) = walked(tmp.path());
    let inside = inv.find("link-in.ts").unwrap();
    assert_eq!(inside.class, FileClass::Symlink);
    let target = inside.symlink_target.as_ref().unwrap();
    assert!(!target.escapes_root);
    assert_eq!(target.relative.as_ref().unwrap().as_str(), "src/app.ts");

    for escaping in ["link-out", "link-up"] {
        let e = inv.find(escaping).unwrap();
        assert_eq!(e.class, FileClass::Symlink);
        assert!(
            e.symlink_target.as_ref().unwrap().escapes_root,
            "{escaping}"
        );
    }
    let warned: Vec<&str> = warnings
        .iter()
        .filter(|w| w.code == "symlink_escapes_root")
        .filter_map(|w| w.path.as_ref().map(|p| p.as_str()))
        .collect();
    assert_eq!(warned, vec!["link-out", "link-up"]);
}

#[test]
fn sensitive_files_classified_and_never_read() {
    let tmp = edge_case_tree();
    let counting = Arc::new(CountingFiles::default());
    let (inv, warnings) = walk_with(tmp.path(), &WalkOptions::default(), counting.clone()).unwrap();
    for sensitive in [
        "gcs-key.json",
        ".env.production",
        ".env.test",
        "certs/server.pem",
    ] {
        assert_eq!(
            class_of(&inv, sensitive),
            FileClass::Sensitive,
            "{sensitive}"
        );
    }
    let opened = counting.opened.lock().unwrap();
    for name in ["gcs-key.json", ".env.production", ".env.test", "server.pem"] {
        assert!(
            opened.iter().all(|p| !p.ends_with(name)),
            "{name} must never be opened"
        );
    }
    let json = serde_json::to_string(&inv).unwrap();
    let warning_text = format!("{warnings:?}");
    for canary in CANARIES {
        assert!(!json.contains(canary));
        assert!(!warning_text.contains(canary));
    }
    let reader = BoundedReader::new(tmp.path());
    let entry = inv.find(".env.production").unwrap();
    assert!(matches!(
        reader.read_text(entry, 100),
        Err(ReadError::Refused(FileClass::Sensitive))
    ));
}

#[test]
fn env_template_is_source_not_sensitive() {
    let tmp = edge_case_tree();
    let (inv, _) = walked(tmp.path());
    assert_eq!(class_of(&inv, ".env.example"), FileClass::Source);
}

#[test]
fn case_collision_warned() {
    let tmp = edge_case_tree();
    let (_, warnings) = walked(tmp.path());
    let collisions: Vec<_> = warnings
        .iter()
        .filter(|w| w.code == "case_collision")
        .collect();
    assert_eq!(collisions.len(), 1, "{warnings:?}");
    assert!(collisions[0].message.contains("README.md"));
    assert!(collisions[0].message.contains("Readme.md"));
}

#[cfg(unix)]
#[test]
fn non_utf8_path_skipped_with_warning() {
    let tmp = edge_case_tree();
    let (inv, warnings) = walked(tmp.path());
    assert!(warnings.iter().any(|w| w.code == "non_utf8_path"));
    assert!(inv
        .entries
        .iter()
        .all(|e| !e.path.as_str().contains("bad-")));
    assert_eq!(
        inv.ignored
            .get(&repository::walk::IgnoreReason::NonUtf8Path),
        Some(&1)
    );
}

#[test]
fn too_many_files_is_explicit_error() {
    let tmp = edge_case_tree();
    let opts = WalkOptions {
        max_files: 5,
        ..WalkOptions::default()
    };
    let err = walk(tmp.path(), &opts).unwrap_err();
    assert!(
        matches!(err, InitError::TooManyFiles { limit: 5 }),
        "{err:?}"
    );
}

#[test]
fn output_sorted_and_deterministic_across_thread_counts() {
    let tmp = edge_case_tree();
    let run = |threads| {
        let opts = WalkOptions {
            threads: Some(threads),
            ..WalkOptions::default()
        };
        let (inv, warnings) = walk(tmp.path(), &opts).unwrap();
        (serde_json::to_string(&inv).unwrap(), warnings)
    };
    let one = run(1);
    let eight = run(8);
    assert_eq!(one, eight);
    let (inv, _) = walked(tmp.path());
    let mut sorted: Vec<_> = inv.entries.iter().map(|e| e.path.clone()).collect();
    let before = sorted.clone();
    sorted.sort();
    assert_eq!(before, sorted);
}

#[test]
fn unreadable_root_is_an_io_error() {
    let err = walk(Path::new("/definitely/not/here"), &WalkOptions::default()).unwrap_err();
    assert!(matches!(err, InitError::Io { .. }));
}

#[test]
fn bounded_reader_reads_source_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "a.txt", "hello world");
    let (inv, _) = walked(tmp.path());
    let reader = BoundedReader::new(&inv.root);
    let entry = inv.find("a.txt").unwrap();
    assert_eq!(reader.read_text(entry, 5).unwrap(), "hello");
}

/// `std::fs::read` and `File::open` may only appear in the walker and the reader.
#[test]
fn bounded_reader_is_the_only_content_read_path() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || matches!(name.as_str(), "walk.rs" | "read.rs" | "git.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for forbidden in ["std::fs::read(", "File::open", "fs::read_to_string"] {
            assert!(
                !text.contains(forbidden),
                "{name} reads files directly with `{forbidden}`; use BoundedReader"
            );
        }
    }
}

proptest! {
    #[test]
    fn repo_path_rejects_traversal(prefix in "[a-z]{1,5}", suffix in "[a-z]{1,5}") {
        let up = format!("{prefix}/../{suffix}");
        prop_assert!(RepoPath::new(up).is_err());
        let abs = format!("/{prefix}/{suffix}");
        prop_assert!(RepoPath::new(abs).is_err());
        let back = format!("{prefix}\\{suffix}");
        prop_assert!(RepoPath::new(back).is_err());
        let nul = format!("{prefix}\0{suffix}");
        prop_assert!(RepoPath::new(nul).is_err());
        let ok = format!("{prefix}/{suffix}");
        prop_assert!(RepoPath::new(ok).is_ok());
    }
}

/// Acceptance evidence for M2; needs RG_REFERENCE_REPO_PATH. Never CI-required.
#[test]
#[ignore = "needs RG_REFERENCE_REPO_PATH"]
fn walk_reference_api() {
    let root = std::env::var("RG_REFERENCE_REPO_PATH").expect("RG_REFERENCE_REPO_PATH");
    let (inv, _) = walk(Path::new(&root), &WalkOptions::default()).unwrap();
    let sources = inv
        .entries
        .iter()
        .filter(|e| e.class == FileClass::Source)
        .count();
    assert!((1_000..=1_200).contains(&sources), "{sources} source files");
    if let Some(entry) = inv.find("gcs-key.json") {
        assert_eq!(entry.class, FileClass::Sensitive);
    }
}
