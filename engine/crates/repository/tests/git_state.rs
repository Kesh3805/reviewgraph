#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use repository::git::{discover, DefaultBranchSource, GitOpenOptions, HeadState};
use repository::remote_url::ProviderHint;
use repository::InitError;
use review_test_support::{add_origin, empty_repo, fixture_copy, fixture_repo, git, write_file};

fn opts() -> GitOpenOptions {
    GitOpenOptions::default()
}

#[test]
fn discover_from_nested_subdir_finds_root() {
    let repo = fixture_repo("init-basic");
    let found = discover(&repo.join("src"), &opts()).unwrap();
    assert_eq!(found.root, std::fs::canonicalize(&repo).unwrap());
    assert!(found.git.is_some());
}

#[test]
fn head_on_branch_reports_sha_and_branch() {
    let repo = fixture_repo("init-basic");
    let state = discover(&repo, &opts()).unwrap().git.unwrap();
    let expected = git(&repo, &["rev-parse", "HEAD"]);
    match &state.head {
        HeadState::Commit { sha, branch } => {
            assert_eq!(sha.as_str(), expected);
            assert_eq!(branch.as_deref(), Some("main"));
        }
        other => panic!("unexpected head {other:?}"),
    }
}

#[test]
fn detached_head_reports_detached() {
    let tmp = fixture_copy("init-basic");
    git(tmp.path(), &["checkout", "-q", "--detach", "step-002"]);
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    let expected = git(tmp.path(), &["rev-parse", "step-002"]);
    match state.head {
        HeadState::Detached { sha } => assert_eq!(sha.as_str(), expected),
        other => panic!("unexpected head {other:?}"),
    }
}

#[test]
fn unborn_head_in_empty_repo() {
    let tmp = empty_repo();
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    assert_eq!(
        state.head,
        HeadState::Unborn {
            branch: Some("main".to_owned())
        }
    );
    assert_eq!(state.default_branch.as_deref(), Some("main"));
    assert_eq!(
        state.default_branch_source,
        DefaultBranchSource::CurrentBranch
    );
}

#[test]
fn bare_repo_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "--bare", "-b", "main"]);
    let err = discover(tmp.path(), &opts()).unwrap_err();
    assert!(matches!(err, InitError::BareRepository(_)), "{err:?}");
}

#[test]
fn non_git_dir_rejected_unless_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let err = discover(tmp.path(), &opts()).unwrap_err();
    assert!(matches!(err, InitError::NotAGitRepository(_)), "{err:?}");

    let allowed = discover(
        tmp.path(),
        &GitOpenOptions {
            allow_non_git: true,
            ..GitOpenOptions::default()
        },
    )
    .unwrap();
    assert!(allowed.git.is_none());
    assert_eq!(allowed.warnings[0].code, "not_a_git_repository");
}

#[test]
fn default_branch_from_origin_head() {
    let tmp = fixture_copy("init-basic");
    add_origin(tmp.path(), "https://github.com/acme/init-basic.git", "main");
    git(tmp.path(), &["branch", "-m", "main", "trunk-local"]);
    // origin/HEAD wins over the well-known/current branch rules.
    git(
        tmp.path(),
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    assert_eq!(state.default_branch.as_deref(), Some("main"));
    assert_eq!(state.default_branch_source, DefaultBranchSource::OriginHead);
    assert_eq!(state.remotes.len(), 1);
    assert_eq!(state.remotes[0].provider, ProviderHint::Github);
}

#[test]
fn default_branch_falls_back_to_main_then_current() {
    let tmp = fixture_copy("init-basic");
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    assert_eq!(state.default_branch.as_deref(), Some("main"));
    assert_eq!(
        state.default_branch_source,
        DefaultBranchSource::WellKnownName
    );

    let provider = discover(
        tmp.path(),
        &GitOpenOptions {
            provider_default_branch: Some("release".to_owned()),
            ..GitOpenOptions::default()
        },
    )
    .unwrap()
    .git
    .unwrap();
    assert_eq!(provider.default_branch.as_deref(), Some("release"));
    assert_eq!(
        provider.default_branch_source,
        DefaultBranchSource::Provider
    );

    // No well-known branch left: the current branch decides.
    git(tmp.path(), &["branch", "-m", "main", "work"]);
    git(tmp.path(), &["branch", "-D", "feature/x"]);
    let current = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    assert_eq!(current.default_branch.as_deref(), Some("work"));
    assert_eq!(
        current.default_branch_source,
        DefaultBranchSource::CurrentBranch
    );
}

#[test]
fn remote_url_credentials_are_redacted() {
    let tmp = fixture_copy("init-basic");
    git(
        tmp.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://deploy-user:ghp_SECRETTOKEN1234@github.com/acme/widgets.git?x=1",
        ],
    );
    git(
        tmp.path(),
        &[
            "remote",
            "add",
            "ci",
            "https://x-access-token:ghs_OTHERTOKEN9876@github.com/acme/widgets.git",
        ],
    );
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    assert_eq!(state.remotes.len(), 2);
    assert_eq!(state.remotes[0].name, "ci");
    assert_eq!(state.remotes[1].name, "origin");
    let debug = format!("{state:?}");
    let json = serde_json::to_string(&state).unwrap();
    for secret in [
        "ghp_SECRETTOKEN1234",
        "ghs_OTHERTOKEN9876",
        "deploy-user",
        "x-access-token",
    ] {
        assert!(!debug.contains(secret), "{secret} in Debug output");
        assert!(!json.contains(secret), "{secret} in JSON output");
    }
    assert_eq!(
        state.remotes[1].url.as_str(),
        "https://github.com/acme/widgets.git"
    );
}

#[test]
fn scp_like_ssh_url_slug_parsed() {
    let tmp = fixture_copy("init-basic");
    git(
        tmp.path(),
        &["remote", "add", "origin", "git@gitlab.com:group/proj.git"],
    );
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    let remote = &state.remotes[0];
    assert_eq!(remote.url.as_str(), "git@gitlab.com:group/proj.git");
    assert_eq!(remote.provider, ProviderHint::Gitlab);
    assert_eq!(remote.slug.as_deref(), Some("gitlab.com/group/proj"));
}

#[test]
fn dirty_counts_staged_unstaged_untracked() {
    let tmp = fixture_copy("init-basic");
    let clean = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    assert!(!clean.dirty.is_dirty, "{:?}", clean.dirty);

    // staged: a new file added to the index
    write_file(tmp.path(), "src/staged.ts", "export const a = 1;\n");
    git(tmp.path(), &["add", "src/staged.ts"]);
    // unstaged: modify a tracked file
    write_file(tmp.path(), "src/index.ts", "export const changed = 2;\n");
    // untracked: two new files
    write_file(tmp.path(), "src/u1.ts", "1\n");
    write_file(tmp.path(), "docs/u2.md", "2\n");

    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    let dirty = &state.dirty;
    assert!(dirty.is_dirty);
    assert_eq!(dirty.staged, 1, "{dirty:?}");
    assert_eq!(dirty.unstaged, 1, "{dirty:?}");
    assert_eq!(dirty.untracked, 2, "{dirty:?}");
    let paths: Vec<&str> = dirty.sample_paths.iter().map(|p| p.as_str()).collect();
    assert_eq!(
        paths,
        vec!["docs/u2.md", "src/index.ts", "src/staged.ts", "src/u1.ts"]
    );
    assert!(!dirty.truncated);
}

#[test]
fn dirty_sample_paths_are_capped_and_sorted() {
    let tmp = fixture_copy("init-basic");
    for i in 0..30 {
        write_file(tmp.path(), &format!("scratch/f{i:02}.txt"), "x\n");
    }
    let dirty = discover(tmp.path(), &opts()).unwrap().git.unwrap().dirty;
    assert_eq!(dirty.untracked, 30);
    assert_eq!(dirty.sample_paths.len(), 20);
    let mut sorted = dirty.sample_paths.clone();
    sorted.sort();
    assert_eq!(sorted, dirty.sample_paths);
    assert_eq!(dirty.sample_paths[0].as_str(), "scratch/f00.txt");
}

#[test]
fn global_git_config_is_ignored() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join(".gitconfig"),
        "[remote \"leaked\"]\n\turl = https://example.com/leaked/repo.git\n",
    )
    .unwrap();
    let tmp = fixture_copy("init-basic");
    let previous_home = std::env::var_os("HOME");
    let previous_xdg = std::env::var_os("XDG_CONFIG_HOME");
    // SAFETY-equivalent: this test binary runs this process-wide env change only here.
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path());
    let result = discover(tmp.path(), &opts());
    match previous_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    match previous_xdg {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let state = result.unwrap().git.unwrap();
    assert!(state.remotes.is_empty(), "{:?}", state.remotes);
}

#[test]
fn submodule_paths_listed_not_recursed() {
    let tmp = fixture_copy("init-basic");
    write_file(
        tmp.path(),
        ".gitmodules",
        "[submodule \"vendor/lib\"]\n\tpath = vendor/lib\n\turl = https://example.com/lib.git\n[submodule \"a/b\"]\n\tpath = a/b\n\turl = https://example.com/b.git\n",
    );
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    let paths: Vec<&str> = state.submodules.iter().map(|p| p.as_str()).collect();
    assert_eq!(paths, vec!["a/b", "vendor/lib"]);
}

#[test]
fn lfs_patterns_from_gitattributes() {
    let tmp = fixture_copy("init-basic");
    write_file(
        tmp.path(),
        ".gitattributes",
        "# comment\n*.psd filter=lfs diff=lfs merge=lfs -text\n*.ts text eol=lf\nassets/** filter=lfs\n",
    );
    let state = discover(tmp.path(), &opts()).unwrap().git.unwrap();
    assert_eq!(state.lfs_patterns, vec!["*.psd", "assets/**"]);
}

#[test]
fn two_calls_on_an_unchanged_repo_are_equal() {
    let repo = fixture_repo("init-basic");
    let a = discover(&repo, &opts()).unwrap();
    let b = discover(&repo, &opts()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn linked_worktree_is_flagged() {
    let tmp = fixture_copy("init-basic");
    let wt = tempfile::tempdir().unwrap();
    let wt_path = wt.path().join("linked");
    git(
        tmp.path(),
        &[
            "worktree",
            "add",
            "-q",
            wt_path.to_str().unwrap(),
            "feature/x",
        ],
    );
    let state = discover(&wt_path, &opts()).unwrap().git.unwrap();
    assert!(state.is_linked_worktree);
    assert_eq!(state.head.branch(), Some("feature/x"));
}
