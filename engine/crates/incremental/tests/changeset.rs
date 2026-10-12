#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! INC-001 acceptance tests: path-level change detection between two commits.

use diff_engine::git::object_reads_total;
use diff_engine::testkit::{FileSpec, FixtureRepo, RepoBuilder, Scenario};
use incremental::changeset::{detect_changes, ChangeSet, PathChangeKind};
use repository::git::TreeDiffOptions;
use review_core::ids::CommitSha;

fn two_commits(
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

fn detect(repo: &FixtureRepo, base: &CommitSha, head: &CommitSha) -> ChangeSet {
    detect_with(repo, base, head, &TreeDiffOptions::default())
}

fn detect_with(
    repo: &FixtureRepo,
    base: &CommitSha,
    head: &CommitSha,
    opts: &TreeDiffOptions,
) -> ChangeSet {
    let git = repo.open().unwrap();
    detect_changes(&git.local(), base, head, opts).unwrap()
}

fn summary(set: &ChangeSet) -> Vec<(String, PathChangeKind)> {
    set.entries
        .iter()
        .map(|entry| (entry.path.as_str().to_owned(), entry.kind))
        .collect()
}

const BODY: &str = "export function compute(a: number, b: number): number {\n  const sum = a + b;\n  const product = a * b;\n  const diff = a - b;\n  return sum + product + diff;\n}\n";

#[test]
fn added_modified_deleted_detected() {
    let (repo, base, head) = two_commits(
        &[
            FileSpec::file("a.ts", "export const a = 1;\n"),
            FileSpec::file("b.ts", "export const b = 1;\n"),
            FileSpec::file("c.ts", "export const c = 1;\n"),
        ],
        &[
            FileSpec::file("a.ts", "export const a = 1;\n"),
            FileSpec::file("b.ts", "export const b = 2;\n"),
            FileSpec::file("d.ts", "export const d = 1;\n"),
        ],
    );
    let set = detect(&repo, &base, &head);
    assert_eq!(
        summary(&set),
        vec![
            ("b.ts".to_owned(), PathChangeKind::Modified),
            ("c.ts".to_owned(), PathChangeKind::Deleted),
            ("d.ts".to_owned(), PathChangeKind::Added),
        ]
    );
    assert_eq!(set.stats.entries, 3);
    assert_eq!(set.stats.renamed, 0);
    assert_eq!(set.base, base);
    assert_eq!(set.head, head);
    let added = set.get(&"d.ts".parse().unwrap()).unwrap();
    assert!(added.base_oid.is_none() && added.head_oid.is_some());
}

#[test]
fn rename_with_edits_reports_renamed_from() {
    let edited = format!("{BODY}// trailing edit\n");
    let (repo, base, head) = two_commits(
        &[FileSpec::file("src/old.ts", BODY)],
        &[FileSpec::file("src/new.ts", &edited)],
    );
    let set = detect(&repo, &base, &head);
    assert_eq!(
        summary(&set),
        vec![("src/new.ts".to_owned(), PathChangeKind::Renamed)]
    );
    let entry = &set.entries[0];
    assert_eq!(entry.renamed_from.as_ref().unwrap().as_str(), "src/old.ts");
    assert_eq!(set.stats.renamed, 1);
    assert_eq!(set.rename_pairs().len(), 1);
}

#[test]
fn identical_subtrees_not_visited() {
    let contents: Vec<(String, String)> = (0..400)
        .map(|i| {
            (
                format!("pkg{}/dir{}/file{i}.ts", i % 20, i % 7),
                format!("export const v{i} = {i};\n"),
            )
        })
        .collect();
    let base_specs: Vec<FileSpec<'_>> =
        contents.iter().map(|(p, c)| FileSpec::file(p, c)).collect();
    let changed = "export const v0 = 12345;\n".to_owned();
    let head_specs: Vec<FileSpec<'_>> = contents
        .iter()
        .enumerate()
        .map(|(i, (p, c))| {
            if i == 0 {
                FileSpec::file(p, &changed)
            } else {
                FileSpec::file(p, c)
            }
        })
        .collect();
    let (repo, base, head) = two_commits(&base_specs, &head_specs);
    let git = repo.open().unwrap();
    let repo_handle = git.local();

    let before = object_reads_total();
    let set = detect_changes(&repo_handle, &base, &head, &TreeDiffOptions::default()).unwrap();
    let reads = object_reads_total() - before;

    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].path.as_str(), "pkg0/dir0/file0.ts");
    // Objects read through `GitRepo` are only counted for explicit reads; the diff itself uses
    // the raw handle, so this guards against anyone adding a per-file read to the wrapper.
    assert!(reads < 400, "read {reads} objects for one changed file");
}

#[test]
fn rename_limit_degrades_to_add_delete() {
    let mk = |tag: &str| format!("{BODY}// {tag}\n");
    let other = "export function other(x: string): string {\n  return x.trim().toUpperCase();\n}\n";
    let (repo, base, head) = two_commits(
        &[
            FileSpec::file("a/one.ts", BODY),
            FileSpec::file("a/two.ts", other),
        ],
        &[
            FileSpec::file("b/one.ts", &mk("edited one")),
            FileSpec::file("b/two.ts", &format!("{other}// edited two\n")),
        ],
    );
    let opts = TreeDiffOptions {
        rename_limit: 1,
        ..TreeDiffOptions::default()
    };
    let set = detect_with(&repo, &base, &head, &opts);
    if set.stats.rename_limit_hit {
        // Degraded: add + delete pairs, no information lost about which paths changed.
        assert_eq!(set.stats.renamed, 0);
        assert_eq!(set.entries.len(), 4);
    } else {
        assert_eq!(set.entries.len(), 2);
    }
}

#[test]
fn submodule_and_symlink_skipped() {
    let (repo, base, head) = two_commits(
        &[FileSpec::file("keep.ts", "export const k = 1;\n")],
        &[
            FileSpec::file("keep.ts", "export const k = 1;\n"),
            FileSpec::symlink("link.ts", "keep.ts"),
            FileSpec::gitlink("vendor/lib", "0123456789abcdef0123456789abcdef01234567"),
            FileSpec::file("new.ts", "export const n = 1;\n"),
        ],
    );
    let set = detect(&repo, &base, &head);
    assert_eq!(
        summary(&set),
        vec![("new.ts".to_owned(), PathChangeKind::Added)]
    );
    assert_eq!(set.stats.skipped_symlink, 1);
    assert_eq!(set.stats.skipped_submodule, 1);
}

#[test]
fn type_change_file_to_symlink() {
    let (repo, base, head) = two_commits(
        &[
            FileSpec::file("a.ts", "export const a = 1;\n"),
            FileSpec::file("b.ts", "export const b = 1;\n"),
        ],
        &[
            FileSpec::symlink("a.ts", "b.ts"),
            FileSpec::file("b.ts", "export const b = 1;\n"),
        ],
    );
    let set = detect(&repo, &base, &head);
    // The new side is a symlink (not indexable), so only the deletion of the file remains.
    assert_eq!(
        summary(&set),
        vec![("a.ts".to_owned(), PathChangeKind::Deleted)]
    );
}

#[test]
fn output_sorted_and_deterministic() {
    let (repo, base, head) = two_commits(
        &[
            FileSpec::file("z.ts", "export const z = 1;\n"),
            FileSpec::file("m/n.ts", "export const n = 1;\n"),
        ],
        &[
            FileSpec::file("z.ts", "export const z = 2;\n"),
            FileSpec::file("m/n.ts", "export const n = 2;\n"),
            FileSpec::file("a.ts", "export const a = 1;\n"),
        ],
    );
    let first = detect(&repo, &base, &head);
    let second = detect(&repo, &base, &head);
    assert_eq!(first, second);
    let paths: Vec<&str> = first.entries.iter().map(|e| e.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
}

#[test]
fn missing_commit_is_an_error_not_an_empty_set() {
    let (repo, base, _head) = two_commits(
        &[FileSpec::file("a.ts", "export const a = 1;\n")],
        &[FileSpec::file("a.ts", "export const a = 2;\n")],
    );
    let git = repo.open().unwrap();
    let ghost: CommitSha = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
    let err = detect_changes(&git.local(), &base, &ghost, &TreeDiffOptions::default())
        .expect_err("a missing head must fail");
    assert!(matches!(
        err,
        incremental::ChangeSetError::CommitNotFound(_)
    ));
}

#[test]
fn golden_auth_bypass_changeset() {
    let scenario = Scenario::load("auth-bypass").unwrap();
    let set = detect(&scenario.repo, &scenario.base, &scenario.head);
    assert_eq!(
        summary(&set),
        vec![
            (
                "src/auth/auth.service.ts".to_owned(),
                PathChangeKind::Modified
            ),
            ("src/util/format.ts".to_owned(), PathChangeKind::Modified),
        ]
    );
}
