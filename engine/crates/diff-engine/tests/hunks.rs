#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::single_range_in_vec_init
)]

//! DIFF-003 acceptance tests: in-process histogram line diff.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use diff_engine::files::{diff_commits, DiffOptions};
use diff_engine::hunks::{
    apply_hunks, compute_hunks, DiffHunk, HunkError, HunkOptions, LineKind, LineStats,
};
use diff_engine::model::DiffModel;
use diff_engine::testkit::{FileSpec, RepoBuilder, Scenario};
use proptest::prelude::*;
use review_core::change::{FileChangeStatus, Hunk};

fn opts() -> HunkOptions {
    HunkOptions::default()
}

fn header(old_start: u32, old_lines: u32, new_start: u32, new_lines: u32) -> Hunk {
    Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    }
}

fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

/// Render hunks the way `git diff` does (count omitted when it is 1).
fn render(hunks: &[DiffHunk], old: &[u8], new: &[u8]) -> String {
    fn range(start: u32, lines: u32) -> String {
        if lines == 1 {
            format!("{start}")
        } else {
            format!("{start},{lines}")
        }
    }
    let mut out = String::new();
    for hunk in hunks {
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            range(hunk.header.old_start, hunk.header.old_lines),
            range(hunk.header.new_start, hunk.header.new_lines)
        ));
        for line in &hunk.lines {
            let (prefix, buf) = match line.kind {
                LineKind::Context => (' ', new),
                LineKind::Add => ('+', new),
                LineKind::Del => ('-', old),
            };
            let text = &buf[line.span.start as usize..line.span.end as usize];
            out.push(prefix);
            out.push_str(&String::from_utf8_lossy(text));
            if line.no_eol {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    out
}

/// The hunk part of each file section of a `git diff` output, keyed by new path, with the
/// function-context suffix of `@@` lines removed.
fn git_hunks_by_file(patch: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let new = rest.split(' ').nth(1).unwrap_or("");
            out.push((new.trim_start_matches("b/").to_owned(), String::new()));
            continue;
        }
        let Some((_, body)) = out.last_mut() else {
            continue;
        };
        if let Some(after) = line.strip_prefix("@@ ") {
            let end = after.find(" @@").map(|i| i + 3 + 3).unwrap_or(line.len());
            body.push_str(&line[..end]);
            body.push('\n');
        } else if !body.is_empty() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out
}

#[test]
fn simple_modification_range() {
    let old = numbered(10);
    let new = old.replace("line 5\n", "line five\n");
    let set = compute_hunks(old.as_bytes(), new.as_bytes(), &opts()).unwrap();
    assert_eq!(set.hunks.len(), 1);
    assert_eq!(set.hunks[0].header, header(2, 7, 2, 7));
    assert_eq!(set.changed_old, vec![5..6]);
    assert_eq!(set.changed_new, vec![5..6]);
    assert_eq!(
        set.stats,
        LineStats {
            additions: 1,
            deletions: 1
        }
    );
    let kinds: Vec<LineKind> = set.hunks[0].lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            LineKind::Context,
            LineKind::Context,
            LineKind::Context,
            LineKind::Del,
            LineKind::Add,
            LineKind::Context,
            LineKind::Context,
            LineKind::Context
        ]
    );
}

#[test]
fn pure_insertion_and_deletion_headers() {
    let old = numbered(3);
    let zero = HunkOptions {
        context: 0,
        ..opts()
    };
    // Insertion after line 2.
    let inserted = "line 1\nline 2\nnew\nline 3\n";
    let set = compute_hunks(old.as_bytes(), inserted.as_bytes(), &zero).unwrap();
    assert_eq!(set.hunks[0].header, header(2, 0, 3, 1));
    // Insertion at the start of the file.
    let at_start = format!("new\n{old}");
    let set = compute_hunks(old.as_bytes(), at_start.as_bytes(), &zero).unwrap();
    assert_eq!(set.hunks[0].header, header(0, 0, 1, 1));
    // Deletion of line 2.
    let deleted = "line 1\nline 3\n";
    let set = compute_hunks(old.as_bytes(), deleted.as_bytes(), &zero).unwrap();
    assert_eq!(set.hunks[0].header, header(2, 1, 1, 0));
    assert_eq!(set.changed_old, vec![2..3]);
    assert!(set.changed_new.is_empty());
}

#[test]
fn context_merging_adjacent_hunks() {
    let old = numbered(30);
    // Changes 6 lines apart (gap 5 <= 2*3): one hunk.
    let near = old
        .replace("line 5\n", "five\n")
        .replace("line 11\n", "eleven\n");
    let set = compute_hunks(old.as_bytes(), near.as_bytes(), &opts()).unwrap();
    assert_eq!(set.hunks.len(), 1);
    assert_eq!(set.hunks[0].header, header(2, 13, 2, 13));
    // Changes 8 lines apart (gap 7 > 6): two hunks.
    let far = old
        .replace("line 5\n", "five\n")
        .replace("line 13\n", "thirteen\n");
    let set = compute_hunks(old.as_bytes(), far.as_bytes(), &opts()).unwrap();
    assert_eq!(set.hunks.len(), 2);
    assert_eq!(set.changed_new, vec![5..6, 13..14]);
}

#[test]
fn crlf_vs_lf_is_a_change_unless_ignored() {
    let old = "a\nb\nc\n";
    let new = "a\r\nb\r\nc\r\n";
    let set = compute_hunks(old.as_bytes(), new.as_bytes(), &opts()).unwrap();
    assert_eq!(set.stats.additions, 3);
    assert_eq!(set.stats.deletions, 3);
    let ignoring = HunkOptions {
        ignore_eol: true,
        ..opts()
    };
    let set = compute_hunks(old.as_bytes(), new.as_bytes(), &ignoring).unwrap();
    assert!(set.hunks.is_empty());
}

#[test]
fn no_newline_at_eof_detected() {
    let old = "a\nb\n";
    let new = "a\nb";
    let set = compute_hunks(old.as_bytes(), new.as_bytes(), &opts()).unwrap();
    assert_eq!(set.stats.additions, 1);
    assert_eq!(set.stats.deletions, 1);
    let added = set.hunks[0]
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Add)
        .unwrap();
    assert!(added.no_eol);
    let removed = set.hunks[0]
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Del)
        .unwrap();
    assert!(!removed.no_eol);
}

fn two_commit_model(base: &[FileSpec<'_>], head: &[FileSpec<'_>]) -> DiffModel {
    let mut repo = RepoBuilder::worktree().unwrap();
    let b = repo.commit("base", &[], base).unwrap();
    let h = repo.commit("head", std::slice::from_ref(&b), head).unwrap();
    let git = repo.open().unwrap();
    diff_commits(&git, &b, &h, &DiffOptions::default()).unwrap()
}

#[test]
fn empty_old_side_for_added_file() {
    let model = two_commit_model(
        &[FileSpec::file("keep.txt", "k\n")],
        &[
            FileSpec::file("keep.txt", "k\n"),
            FileSpec::file("new.ts", "export const a = 1;\nexport const b = 2;\n"),
        ],
    );
    let file = model.file("new.ts").unwrap();
    assert_eq!(file.file.status, FileChangeStatus::Added);
    assert_eq!(file.hunks.len(), 1);
    assert_eq!(file.hunks[0].header, header(0, 0, 1, 2));
    assert_eq!(file.file.hunks, vec![header(0, 0, 1, 2)]);
    assert_eq!(
        file.lines,
        Some(LineStats {
            additions: 2,
            deletions: 0
        })
    );
}

#[test]
fn rename_with_edit_diffed_against_old_path() {
    let body = numbered(20);
    let edited = body.replace("line 10\n", "line ten\n");
    let model = two_commit_model(
        &[FileSpec::file("src/old.ts", &body)],
        &[FileSpec::file("src/new.ts", &edited)],
    );
    let file = model.file("src/new.ts").unwrap();
    assert_eq!(file.file.status, FileChangeStatus::Renamed);
    assert_eq!(file.hunks.len(), 1);
    assert_eq!(file.hunks[0].header, header(7, 7, 7, 7));
    assert_eq!(
        file.lines,
        Some(LineStats {
            additions: 1,
            deletions: 1
        })
    );
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn matches_git_diff_histogram_u3_for_fixture() {
    for (root, reference) in [
        (
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/diff/auth-bypass"),
            "expected/histogram-u3.diff",
        ),
        (
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../fixtures/pull-requests/auth-bypass"),
            "expected/git-diff-u3.patch",
        ),
    ] {
        let expected = git_hunks_by_file(&read(root.join(reference)));
        assert!(!expected.is_empty());
        for (path, git_text) in expected {
            let (old, new) = if root.join("head").exists() {
                (
                    read(root.join("base").join(&path)),
                    read(root.join("head").join(&path)),
                )
            } else {
                let scenario = Scenario::load("auth-bypass").unwrap();
                (
                    scenario.base_files.get(&path).cloned().unwrap_or_default(),
                    scenario.head_files.get(&path).cloned().unwrap_or_default(),
                )
            };
            let set = compute_hunks(old.as_bytes(), new.as_bytes(), &opts()).unwrap();
            let ours = render(&set.hunks, old.as_bytes(), new.as_bytes());
            assert_eq!(ours, git_text, "hunks differ from git for {path}");
        }
    }
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn unaffected_by_user_git_config() {
    let _lock = match ENV_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let base = "a\nb\nc\nd\n";
    let head = "a\r\nb\nX\nd\n";
    let baseline = two_commit_model(
        &[FileSpec::file("f.txt", base)],
        &[FileSpec::file("f.txt", head)],
    );

    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".gitconfig");
    std::fs::write(
        &config,
        "[diff]\n\talgorithm = patience\n\tnoprefix = true\n\tcontext = 10\n[core]\n\tautocrlf = true\n",
    )
    .unwrap();
    let saved: Vec<(&str, Option<OsString>)> = ["HOME", "GIT_CONFIG_GLOBAL", "XDG_CONFIG_HOME"]
        .iter()
        .map(|k| (*k, std::env::var_os(k)))
        .collect();
    std::env::set_var("HOME", home.path());
    std::env::set_var("GIT_CONFIG_GLOBAL", &config);
    std::env::set_var("XDG_CONFIG_HOME", home.path());
    let configured = two_commit_model(
        &[FileSpec::file("f.txt", base)],
        &[FileSpec::file("f.txt", head)],
    );
    for (key, value) in saved {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    let a = baseline.file("f.txt").unwrap();
    let b = configured.file("f.txt").unwrap();
    assert_eq!(a.hunks, b.hunks);
    assert_eq!(a.lines, b.lines);
    assert_eq!(a.hunks[0].header, header(1, 4, 1, 4));
}

#[test]
fn too_large_input_rejected() {
    let big = numbered(50);
    let small = HunkOptions {
        max_lines: 10,
        ..opts()
    };
    let err = compute_hunks(big.as_bytes(), b"", &small).unwrap_err();
    assert_eq!(err, HunkError::TooLarge { lines: 50, max: 10 });
}

#[test]
fn golden_authorize_hunk_ranges() {
    let scenario = Scenario::load("auth-bypass").unwrap();
    let git = scenario.repo.open().unwrap();
    let model = diff_commits(
        &git,
        &scenario.base,
        &scenario.head,
        &DiffOptions::default(),
    )
    .unwrap();
    let file = model.file("src/auth/auth.service.ts").unwrap();
    assert_eq!(file.file.hunks, vec![header(7, 6, 7, 6)]);
    let removed: Vec<u32> = file.hunks[0]
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Del)
        .filter_map(|l| l.old_no)
        .collect();
    let added: Vec<u32> = file.hunks[0]
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Add)
        .filter_map(|l| l.new_no)
        .collect();
    assert_eq!(removed, vec![10]);
    assert_eq!(added, vec![10]);
}

fn text_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!["a", "b", "c", "  d", "", "e\r", "}"]),
        0..40,
    )
    .prop_flat_map(|lines| {
        let joined = lines.join("\n");
        prop::bool::ANY.prop_map(move |trailing| {
            if trailing && !joined.is_empty() {
                format!("{joined}\n")
            } else {
                joined.clone()
            }
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn apply_hunks_reconstructs_new(old in text_strategy(), new in text_strategy(), context in 0u32..5) {
        let o = HunkOptions { context, ..HunkOptions::default() };
        let set = compute_hunks(old.as_bytes(), new.as_bytes(), &o).unwrap();
        let rebuilt = apply_hunks(old.as_bytes(), new.as_bytes(), &set.hunks);
        prop_assert_eq!(rebuilt, new.as_bytes().to_vec());
        let (changed_old, changed_new) = diff_engine::hunks::changed_ranges(&set.hunks);
        prop_assert_eq!(changed_old, set.changed_old);
        prop_assert_eq!(changed_new, set.changed_new);
    }
}
