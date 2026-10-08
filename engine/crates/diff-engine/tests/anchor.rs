#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! DIFF-005 acceptance tests: provider reconciliation and anchorable-line sets.

use diff_engine::anchor::{
    parse_patch, reconcile, AnchorSource, Discrepancy, LineSet, ProviderFileDiff,
};
use diff_engine::files::{diff_commits, DiffOptions};
use diff_engine::model::DiffModel;
use diff_engine::testkit::{FileSpec, RepoBuilder, Scenario};
use proptest::prelude::*;
use review_core::change::FileChangeStatus;
use review_core::location::{DiffSide, RepoPath};

fn path(p: &str) -> RepoPath {
    RepoPath::new(p).unwrap()
}

fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn model(base: &[FileSpec<'_>], head: &[FileSpec<'_>]) -> DiffModel {
    let mut repo = RepoBuilder::worktree().unwrap();
    let b = repo.commit("base", &[], base).unwrap();
    let h = repo.commit("head", std::slice::from_ref(&b), head).unwrap();
    let git = repo.open().unwrap();
    diff_commits(&git, &b, &h, &DiffOptions::default()).unwrap()
}

fn provider(
    p: &str,
    status: FileChangeStatus,
    counts: (u32, u32),
    patch: Option<&str>,
) -> ProviderFileDiff {
    ProviderFileDiff {
        path: path(p),
        old_path: None,
        status,
        additions: counts.0,
        deletions: counts.1,
        patch: patch.map(str::to_owned),
        truncated_list: false,
    }
}

fn edited_model() -> DiffModel {
    let old = numbered(20);
    let new = old.replace("line 10\n", "line ten\n");
    model(
        &[FileSpec::file("src/a.ts", &old)],
        &[FileSpec::file("src/a.ts", &new)],
    )
}

const A_PATCH: &str =
    "@@ -7,7 +7,7 @@\n line 7\n line 8\n line 9\n-line 10\n+line ten\n line 11\n line 12\n line 13";

#[test]
fn provider_patch_defines_right_and_left_sets() {
    let local = edited_model();
    let out = reconcile(
        &local,
        &[provider(
            "src/a.ts",
            FileChangeStatus::Modified,
            (1, 1),
            Some(A_PATCH),
        )],
    );
    let a = out.anchors.get(&path("src/a.ts")).unwrap();
    assert_eq!(a.source, AnchorSource::ProviderPatch);
    assert!(a.anchorable);
    assert_eq!(a.right.ranges(), &[7..14]);
    assert_eq!(a.left.ranges(), &[7..14]);
    assert!(a.can_anchor(DiffSide::Head, 10));
    assert!(!a.can_anchor(DiffSide::Head, 14));
    assert!(out.discrepancies.is_empty(), "{:?}", out.discrepancies);
}

#[test]
fn local_hunks_used_when_patch_omitted_small_file() {
    let local = edited_model();
    let out = reconcile(
        &local,
        &[provider(
            "src/a.ts",
            FileChangeStatus::Modified,
            (1, 1),
            None,
        )],
    );
    let a = out.anchors.get(&path("src/a.ts")).unwrap();
    assert_eq!(a.source, AnchorSource::LocalHunks);
    assert!(a.anchorable);
    assert_eq!(a.right.ranges(), &[7..14]);
    assert_eq!(
        out.discrepancies,
        vec![Discrepancy::PatchMissing {
            path: path("src/a.ts")
        }]
    );
}

#[test]
fn large_file_without_patch_is_unanchorable() {
    let local = edited_model();
    let out = reconcile(
        &local,
        &[provider(
            "src/a.ts",
            FileChangeStatus::Modified,
            (2_000, 1_500),
            None,
        )],
    );
    let a = out.anchors.get(&path("src/a.ts")).unwrap();
    assert!(!a.anchorable);
    assert_eq!(a.source, AnchorSource::None);
    assert!(!a.can_anchor(DiffSide::Head, 10));
    assert_eq!(a.nearest_within(DiffSide::Head, 10, 3), None);
}

#[test]
fn hunk_boundary_difference_prefers_provider() {
    let local = edited_model();
    // The provider used 2 lines of context.
    let patch = "@@ -8,5 +8,5 @@\n line 8\n line 9\n-line 10\n+line ten\n line 11\n line 12";
    let out = reconcile(
        &local,
        &[provider(
            "src/a.ts",
            FileChangeStatus::Modified,
            (1, 1),
            Some(patch),
        )],
    );
    let a = out.anchors.get(&path("src/a.ts")).unwrap();
    assert_eq!(a.right.ranges(), &[8..13]);
    assert_eq!(
        out.discrepancies,
        vec![Discrepancy::HunkBoundaryDiffers {
            path: path("src/a.ts")
        }]
    );
}

#[test]
fn provider_list_truncation_flagged() {
    let local = model(
        &[
            FileSpec::file("a.txt", "a\n"),
            FileSpec::file("b.txt", "b\n"),
        ],
        &[
            FileSpec::file("a.txt", "A\n"),
            FileSpec::file("b.txt", "B\n"),
        ],
    );
    let mut listed = provider(
        "a.txt",
        FileChangeStatus::Modified,
        (1, 1),
        Some("@@ -1 +1 @@\n-a\n+A"),
    );
    listed.truncated_list = true;
    let out = reconcile(&local, &[listed]);
    assert!(out
        .discrepancies
        .contains(&Discrepancy::ProviderListTruncated));
    assert!(!out
        .discrepancies
        .iter()
        .any(|d| matches!(d, Discrepancy::LocalOnly { .. })));
    let b = out.anchors.get(&path("b.txt")).unwrap();
    assert!(!b.anchorable);
    // Without truncation the unlisted file is reported as local-only.
    let out = reconcile(
        &local,
        &[provider(
            "a.txt",
            FileChangeStatus::Modified,
            (1, 1),
            Some("@@ -1 +1 @@\n-a\n+A"),
        )],
    );
    assert_eq!(
        out.discrepancies,
        vec![Discrepancy::LocalOnly {
            path: path("b.txt")
        }]
    );
}

#[test]
fn rename_anchors_left_on_old_path() {
    let old = numbered(20);
    let new = old.replace("line 10\n", "line ten\n");
    let local = model(
        &[FileSpec::file("src/old.ts", &old)],
        &[FileSpec::file("src/new.ts", &new)],
    );
    let mut pf = provider(
        "src/new.ts",
        FileChangeStatus::Renamed,
        (1, 1),
        Some(A_PATCH),
    );
    pf.old_path = Some(path("src/old.ts"));
    let out = reconcile(&local, &[pf]);
    let a = out.anchors.get(&path("src/new.ts")).unwrap();
    assert_eq!(a.old_path, Some(path("src/old.ts")));
    assert!(a.can_anchor(DiffSide::Base, 10));
    assert!(out.discrepancies.is_empty(), "{:?}", out.discrepancies);
}

#[test]
fn deleted_line_anchors_left_only() {
    let patch = "@@ -1,3 +1,2 @@\n a\n-b\n c";
    let parsed = parse_patch(patch).unwrap();
    assert_eq!(parsed.left.ranges(), &[1..4]);
    assert_eq!(parsed.right.ranges(), &[1..3]);
    let local = model(
        &[FileSpec::file("f.txt", "a\nb\nc\n")],
        &[FileSpec::file("f.txt", "a\nc\n")],
    );
    let out = reconcile(
        &local,
        &[provider(
            "f.txt",
            FileChangeStatus::Modified,
            (0, 1),
            Some(patch),
        )],
    );
    let a = out.anchors.get(&path("f.txt")).unwrap();
    assert!(a.can_anchor(DiffSide::Base, 2));
    // New-side line 2 is `c`, a context line; there is no new-side line 3.
    assert!(!a.can_anchor(DiffSide::Head, 3));
}

#[test]
fn nearest_within_respects_distance_and_side() {
    let local = edited_model();
    let out = reconcile(
        &local,
        &[provider(
            "src/a.ts",
            FileChangeStatus::Modified,
            (1, 1),
            Some(A_PATCH),
        )],
    );
    let a = out.anchors.get(&path("src/a.ts")).unwrap();
    assert_eq!(a.nearest_within(DiffSide::Head, 16, 3), Some(13));
    assert_eq!(a.nearest_within(DiffSide::Head, 17, 3), None);
    assert_eq!(a.nearest_within(DiffSide::Base, 4, 3), Some(7));
    assert_eq!(a.nearest_within(DiffSide::Head, 10, 0), Some(10));
}

#[test]
fn lineset_merges_adjacent_ranges() {
    let set = LineSet::from_lines([3, 1, 2, 7, 8, 5]);
    assert_eq!(set.ranges(), &[1..4, 5..6, 7..9]);
    assert!(set.contains(5));
    assert!(!set.contains(6));
    assert_eq!(set.iter().collect::<Vec<_>>(), vec![1, 2, 3, 5, 7, 8]);
}

/// Per-file patch text (from the first `@@`) of a multi-file `git diff`.
fn sections(patch: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let new = rest.split(' ').nth(1).unwrap_or("");
            out.push((new.trim_start_matches("b/").to_owned(), String::new()));
            continue;
        }
        if let Some((_, body)) = out.last_mut() {
            if line.starts_with("@@") || !body.is_empty() {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    out
}

#[test]
fn golden_authorize_anchor_lines() {
    let scenario = Scenario::load("auth-bypass").unwrap();
    let git = scenario.repo.open().unwrap();
    let local = diff_commits(
        &git,
        &scenario.base,
        &scenario.head,
        &DiffOptions::default(),
    )
    .unwrap();
    let patch = std::fs::read_to_string(scenario.dir.join("patch.diff")).unwrap();
    let files: Vec<ProviderFileDiff> = sections(&patch)
        .into_iter()
        .map(|(p, text)| {
            let parsed = parse_patch(&text).unwrap();
            provider(
                &p,
                FileChangeStatus::Modified,
                (parsed.additions, parsed.deletions),
                Some(&text),
            )
        })
        .collect();
    let out = reconcile(&local, &files);
    assert!(out.discrepancies.is_empty(), "{:?}", out.discrepancies);
    let a = out.anchors.get(&path("src/auth/auth.service.ts")).unwrap();
    // The new `return user.role === 'admin'` line is RIGHT-anchorable, the removed
    // `PermissionService.check` line LEFT-anchorable.
    assert!(a.can_anchor(DiffSide::Head, 10));
    assert!(a.can_anchor(DiffSide::Base, 10));
    assert_eq!(a.right.ranges(), &[7..13]);
    assert_eq!(a.left.ranges(), &[7..13]);
}

proptest! {
    #[test]
    fn malformed_patch_does_not_panic(text in ".{0,400}") {
        let _ = parse_patch(&text);
        let with_header = format!("@@ -1,3 +1,3 @@\n{text}");
        let _ = parse_patch(&with_header);
    }

    #[test]
    fn malformed_lines_do_not_panic(lines in prop::collection::vec("[-+ \\\\@a-z0-9,]{0,12}", 0..30)) {
        let _ = parse_patch(&lines.join("\n"));
    }
}
