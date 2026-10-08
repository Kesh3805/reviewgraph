#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! DIFF-002 acceptance tests: file-level three-dot diff with rename/copy detection.

use std::path::{Path, PathBuf};

use diff_engine::files::{diff_commits, DiffOptions};
use diff_engine::model::{DiffModel, FileDiff};
use diff_engine::testkit::{FileSpec, FixtureRepo, RepoBuilder};
use globset::Glob;
use review_core::change::FileChangeStatus;
use review_core::ids::CommitSha;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/diff/auth-bypass")
}

fn status_of<'m>(model: &'m DiffModel, path: &str) -> &'m FileDiff {
    model
        .files
        .iter()
        .find(|f| f.file.path.as_str() == path)
        .unwrap_or_else(|| {
            let have: Vec<String> = model
                .files
                .iter()
                .map(|f| {
                    format!(
                        "{} {:?} sim={:?} old={:?}",
                        f.file.path.as_str(),
                        f.file.status,
                        f.similarity,
                        f.file.old_path.as_ref().map(|p| p.as_str())
                    )
                })
                .collect();
            panic!("expected file {path}; have {have:?}")
        })
}

fn statuses(model: &DiffModel) -> Vec<(String, FileChangeStatus)> {
    model
        .files
        .iter()
        .map(|f| (f.file.path.as_str().to_owned(), f.file.status))
        .collect()
}

fn simple_repo(
    base: &[FileSpec<'_>],
    head: &[FileSpec<'_>],
) -> (FixtureRepo, CommitSha, CommitSha) {
    let mut repo = RepoBuilder::worktree().unwrap();
    let base_sha = repo.commit("base", &[], base).unwrap();
    let head_sha = repo
        .commit("head", std::slice::from_ref(&base_sha), head)
        .unwrap();
    (repo, base_sha, head_sha)
}

#[test]
fn statuses_for_added_modified_deleted() {
    let (repo, base, head) = simple_repo(
        &[
            FileSpec::file("a.txt", "a\n"),
            FileSpec::file("b.txt", "b\n"),
            FileSpec::file("c.txt", "c\n"),
        ],
        &[
            FileSpec::file("a.txt", "a\n"),
            FileSpec::file("b.txt", "b changed\n"),
            FileSpec::file("d.txt", "d\n"),
        ],
    );
    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert_eq!(
        statuses(&model),
        vec![
            ("b.txt".to_owned(), FileChangeStatus::Modified),
            ("c.txt".to_owned(), FileChangeStatus::Deleted),
            ("d.txt".to_owned(), FileChangeStatus::Added),
        ]
    );
    assert_eq!(model.stats.added, 1);
    assert_eq!(model.stats.modified, 1);
    assert_eq!(model.stats.deleted, 1);
    assert_eq!(model.stats.files, 3);
    assert!(status_of(&model, "b.txt").similarity.is_none());
    assert_ne!(
        status_of(&model, "b.txt").base_oid,
        status_of(&model, "b.txt").head_oid
    );
    assert!(status_of(&model, "d.txt").base_oid.is_none());
    assert!(status_of(&model, "c.txt").head_oid.is_none());
}

#[test]
fn rename_at_threshold_50_percent() {
    let ten_old: String = (0..10).map(|i| format!("line {i}\n")).collect();
    let mut ten_new: String = (0..10).map(|i| format!("line {i}\n")).collect();
    ten_new.replace_range(0..7, "CHANGED");
    let (repo, base, head) = simple_repo(
        &[FileSpec::file("old.txt", ten_old.as_str())],
        &[FileSpec::file("new.txt", ten_new.as_str())],
    );
    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert_eq!(model.files.len(), 1);
    let file = status_of(&model, "new.txt");
    assert_eq!(file.file.status, FileChangeStatus::Renamed);
    assert_eq!(
        file.file.old_path.as_ref().map(|p| p.as_str()),
        Some("old.txt")
    );
    let similarity = file.similarity.expect("rename reports similarity");
    assert!(similarity >= 50, "similarity {similarity} should be >= 50");
    assert_eq!(model.stats.renamed, 1);
}

#[test]
fn rename_below_threshold_is_add_delete() {
    let (repo, base, head) = simple_repo(
        &[FileSpec::file(
            "old.txt",
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet\nkilo lima mike november oscar papa quebec romeo sierra tango\n",
        )],
        &[FileSpec::file(
            "new.txt",
            "zulu yankee xray whiskey victor uniform tango sierra romeo papa\noscar november mike lima kilo juliet india golf foxtrot echo delta\n",
        )],
    );
    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert_eq!(
        statuses(&model),
        vec![
            ("new.txt".to_owned(), FileChangeStatus::Added),
            ("old.txt".to_owned(), FileChangeStatus::Deleted),
        ]
    );
    assert_eq!(model.stats.renamed, 0);
}

#[test]
fn exact_rename_without_edit() {
    let (repo, base, head) = simple_repo(
        &[FileSpec::file("old.txt", "same content\nsecond line\n")],
        &[FileSpec::file("new.txt", "same content\nsecond line\n")],
    );
    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert_eq!(model.files.len(), 1);
    let file = status_of(&model, "new.txt");
    assert_eq!(file.file.status, FileChangeStatus::Renamed);
    assert_eq!(file.similarity, Some(100));
    assert_eq!(
        file.file.old_path.as_ref().map(|p| p.as_str()),
        Some("old.txt")
    );
}

#[test]
fn copy_detection_off_by_default() {
    let (repo, base, head) = simple_repo(
        &[FileSpec::file("a.txt", "shared content\nsecond line\n")],
        &[
            FileSpec::file("a.txt", "shared content\nsecond line\n"),
            FileSpec::file("b.txt", "shared content\nsecond line\n"),
        ],
    );
    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert_eq!(
        statuses(&model),
        vec![("b.txt".to_owned(), FileChangeStatus::Added)]
    );
    assert_eq!(model.stats.copied, 0);
}

#[test]
fn copy_detected_when_enabled_from_modified_source() {
    let (repo, base, head) = simple_repo(
        &[FileSpec::file("a.txt", "original content\nsecond line\n")],
        &[
            FileSpec::file("a.txt", "brand new content\nsecond line changed\n"),
            FileSpec::file("b.txt", "brand new content\nsecond line changed\n"),
        ],
    );
    let git = repo.open().unwrap();
    let opts = DiffOptions {
        detect_copies: true,
        ..DiffOptions::default()
    };
    let model = diff_commits(&git, &base, &head, &opts).unwrap();

    let copy = status_of(&model, "b.txt");
    assert_eq!(copy.file.status, FileChangeStatus::Copied);
    assert_eq!(
        copy.file.old_path.as_ref().map(|p| p.as_str()),
        Some("a.txt")
    );
    assert_eq!(copy.similarity, Some(100));
    assert_eq!(
        status_of(&model, "a.txt").file.status,
        FileChangeStatus::Modified
    );
    assert_eq!(model.stats.copied, 1);
}

#[test]
fn three_dot_uses_merge_base_not_base_tip() {
    let mut repo = RepoBuilder::worktree().unwrap();
    let root = repo
        .commit(
            "root",
            &[],
            &[
                FileSpec::file("common.txt", "common\n"),
                FileSpec::file("feature.txt", "feature v1\n"),
            ],
        )
        .unwrap();
    let base_tip = repo
        .commit(
            "base advances",
            std::slice::from_ref(&root),
            &[
                FileSpec::file("common.txt", "common advanced on base\n"),
                FileSpec::file("feature.txt", "feature v1\n"),
            ],
        )
        .unwrap();
    let head = repo
        .commit(
            "head",
            std::slice::from_ref(&root),
            &[
                FileSpec::file("common.txt", "common\n"),
                FileSpec::file("feature.txt", "feature v2\n"),
            ],
        )
        .unwrap();

    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base_tip, &head, &DiffOptions::default()).unwrap();

    assert_eq!(model.merge_base, Some(root));
    assert_eq!(
        statuses(&model),
        vec![("feature.txt".to_owned(), FileChangeStatus::Modified)]
    );
    assert!(
        model
            .files
            .iter()
            .all(|f| f.file.path.as_str() != "common.txt"),
        "base-side advance must not appear in a three-dot diff"
    );
}

#[test]
fn no_merge_base_falls_back_with_flag() {
    let mut repo = RepoBuilder::worktree().unwrap();
    let root_a = repo
        .commit(
            "root a",
            &[],
            &[
                FileSpec::file("a.txt", "old\n"),
                FileSpec::file("b.txt", "only in base\n"),
            ],
        )
        .unwrap();
    let root_b = repo
        .commit(
            "root b",
            &[],
            &[
                FileSpec::file("a.txt", "new\n"),
                FileSpec::file("c.txt", "only in head\n"),
            ],
        )
        .unwrap();

    let git = repo.open().unwrap();
    let model = diff_commits(&git, &root_a, &root_b, &DiffOptions::default()).unwrap();

    assert_eq!(model.merge_base, None);
    assert_eq!(
        statuses(&model),
        vec![
            ("a.txt".to_owned(), FileChangeStatus::Modified),
            ("b.txt".to_owned(), FileChangeStatus::Deleted),
            ("c.txt".to_owned(), FileChangeStatus::Added),
        ]
    );
}

#[test]
fn symlink_and_submodule_skipped() {
    let (repo, base, head) = simple_repo(
        &[FileSpec::file("keep.txt", "v1\n")],
        &[
            FileSpec::file("keep.txt", "v2\n"),
            FileSpec::symlink("link", "../target"),
            FileSpec::gitlink("submodule", "0000000000000000000000000000000000000001"),
        ],
    );
    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert_eq!(
        statuses(&model),
        vec![("keep.txt".to_owned(), FileChangeStatus::Modified)]
    );
    assert_eq!(model.stats.skipped_symlink, 1);
    assert_eq!(model.stats.skipped_submodule, 1);
}

#[test]
fn filters_exclude_vendor_dir() {
    let (repo, base, head) = simple_repo(
        &[
            FileSpec::file("src/app.ts", "export const app = 1;\n"),
            FileSpec::file("vendor/lib.js", "module.exports = 1;\n"),
            FileSpec::file("node_modules/pkg/index.js", "module.exports = 2;\n"),
        ],
        &[
            FileSpec::file("src/app.ts", "export const app = 2;\n"),
            FileSpec::file("vendor/lib.js", "module.exports = 3;\n"),
            FileSpec::file("node_modules/pkg/index.js", "module.exports = 4;\n"),
        ],
    );
    let git = repo.open().unwrap();
    let opts = DiffOptions {
        exclude_globs: vec![
            Glob::new("vendor/**").unwrap(),
            Glob::new("node_modules/**").unwrap(),
        ],
        ..DiffOptions::default()
    };
    let model = diff_commits(&git, &base, &head, &opts).unwrap();

    assert_eq!(
        statuses(&model),
        vec![("src/app.ts".to_owned(), FileChangeStatus::Modified)]
    );
    assert_eq!(model.stats.filtered, 2);
}

#[test]
fn changedfile_invariants_hold() {
    let ten_old: String = (0..10).map(|i| format!("line {i}\n")).collect();
    let mut ten_new: String = (0..10).map(|i| format!("line {i}\n")).collect();
    ten_new.replace_range(0..7, "CHANGED");
    let (repo, base, head) = simple_repo(
        &[
            FileSpec::file("old.txt", ten_old.as_str()),
            FileSpec::file("keep.txt", "keep\n"),
            FileSpec::file("gone.txt", "gone\n"),
        ],
        &[
            FileSpec::file("new.txt", ten_new.as_str()),
            FileSpec::file("keep.txt", "keep changed\n"),
            FileSpec::file("fresh.txt", "fresh\n"),
        ],
    );
    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert!(!model.files.is_empty());
    for file in &model.files {
        let needs_old = matches!(
            file.file.status,
            FileChangeStatus::Renamed | FileChangeStatus::Copied
        );
        assert_eq!(
            needs_old,
            file.file.old_path.is_some(),
            "old_path presence must match status for {}",
            file.file.path.as_str()
        );
    }
}

#[test]
fn output_sorted_and_deterministic() {
    let (repo, base, head) = simple_repo(
        &[FileSpec::file("untouched.txt", "same\n")],
        &[
            FileSpec::file("z.txt", "z\n"),
            FileSpec::file("m/m.txt", "m\n"),
            FileSpec::file("a.txt", "a\n"),
            FileSpec::file("untouched.txt", "same\n"),
        ],
    );
    let git = repo.open().unwrap();
    let first = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();
    let second = diff_commits(&git, &base, &head, &DiffOptions::default()).unwrap();

    assert_eq!(first, second);
    let paths: Vec<&str> = first.files.iter().map(|f| f.file.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted);
    assert_eq!(paths, vec!["a.txt", "m/m.txt", "z.txt"]);
}

#[test]
fn max_files_truncates_and_flags() {
    let names: Vec<String> = (0..5).map(|i| format!("f{i}.txt")).collect();
    let head_specs: Vec<FileSpec<'_>> = names
        .iter()
        .map(|name| FileSpec::file(name, "x\n"))
        .collect();
    let (repo, base, head) = simple_repo(&[], &head_specs);
    let git = repo.open().unwrap();
    let opts = DiffOptions {
        max_files: 2,
        ..DiffOptions::default()
    };
    let model = diff_commits(&git, &base, &head, &opts).unwrap();

    assert_eq!(model.files.len(), 2);
    assert!(model.stats.truncated);
}

fn collect_tree(root: &Path, rel: &Path, out: &mut Vec<(String, String)>) {
    let dir = root.join(rel);
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let rel_path = rel.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            collect_tree(root, &rel_path, out);
        } else {
            let content = std::fs::read_to_string(entry.path()).unwrap();
            let key = rel_path.to_string_lossy().replace('\\', "/");
            out.push((key, content));
        }
    }
    out.sort();
}

fn name_status_line(file: &FileDiff) -> String {
    use FileChangeStatus::*;
    let letter = match file.file.status {
        Added => "A",
        Modified => "M",
        Deleted => "D",
        Renamed => "R",
        Copied => "C",
    };
    match (&file.file.status, &file.file.old_path) {
        (Renamed | Copied, Some(old)) => format!(
            "{letter}{sim}\t{old}\t{new}",
            sim = file.similarity.unwrap_or(100),
            old = old.as_str(),
            new = file.file.path.as_str(),
        ),
        _ => format!("{letter}\t{}", file.file.path.as_str()),
    }
}

#[test]
fn matches_git_name_status_for_fixture() {
    let root = fixture_dir();
    let mut base_files = Vec::new();
    collect_tree(&root.join("base"), Path::new(""), &mut base_files);
    let mut head_files = Vec::new();
    collect_tree(&root.join("head"), Path::new(""), &mut head_files);
    assert!(!base_files.is_empty());
    assert_eq!(base_files.len(), head_files.len());

    let mut repo = RepoBuilder::worktree().unwrap();
    let base_specs: Vec<FileSpec<'_>> = base_files
        .iter()
        .map(|(path, content)| FileSpec::file(path, content))
        .collect();
    let base_sha = repo.commit("base", &[], &base_specs).unwrap();
    let head_specs: Vec<FileSpec<'_>> = head_files
        .iter()
        .map(|(path, content)| FileSpec::file(path, content))
        .collect();
    let head_sha = repo
        .commit("head", std::slice::from_ref(&base_sha), &head_specs)
        .unwrap();

    let git = repo.open().unwrap();
    let model = diff_commits(&git, &base_sha, &head_sha, &DiffOptions::default()).unwrap();
    let actual: Vec<String> = model.files.iter().map(name_status_line).collect();

    let expected_raw = std::fs::read_to_string(root.join("expected/name-status.txt")).unwrap();
    let expected: Vec<&str> = expected_raw.lines().collect();

    // The golden `auth-bypass` fixture contains exactly two modified files.
    assert_eq!(
        expected,
        vec!["M\tsrc/auth/auth.service.ts", "M\tsrc/util/format.ts",]
    );
    let actual_refs: Vec<&str> = actual.iter().map(String::as_str).collect();
    assert_eq!(actual_refs, expected);
    assert_eq!(model.stats.modified, 2);
}
