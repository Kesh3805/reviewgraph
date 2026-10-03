#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use repository::manifests::detect_manifests;
use repository::read::BoundedReader;
use repository::walk::{walk, WalkOptions};
use repository::workspaces::{detect_workspaces, WorkspaceLayout, WorkspaceTool};
use repository::InitWarning;
use review_core::location::RepoPath;
use review_test_support::{fixture_repo, write_file};

fn layout(root: &Path) -> (WorkspaceLayout, Vec<InitWarning>) {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (manifests, _) = detect_manifests(&inv, &reader);
    detect_workspaces(&inv, &reader, &manifests)
}

fn names(l: &WorkspaceLayout) -> Vec<&str> {
    l.packages.iter().map(|p| p.name.as_str()).collect()
}

fn pkg_json(root: &Path, dir: &str, body: &str) {
    write_file(root, &format!("{dir}/package.json"), body);
}

#[test]
fn pnpm_workspace_globs_expand_to_packages() {
    let repo = fixture_repo("monorepo-pnpm");
    let (l, warnings) = layout(&repo);
    assert!(l.is_monorepo);
    assert_eq!(
        names(&l),
        vec!["@acme/api", "@acme/web", "@acme/db", "@acme/shared"]
    );
    assert!(l.tools.contains(&WorkspaceTool::Pnpm));
    assert!(l.tools.contains(&WorkspaceTool::Turbo));
    assert_eq!(l.turbo_tasks, vec!["build", "lint", "test"]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(l.root_package.is_some());
}

#[test]
fn internal_edges_from_workspace_protocol() {
    let repo = fixture_repo("monorepo-pnpm");
    let (l, _) = layout(&repo);
    let edges: Vec<(&str, &str)> = l
        .internal_edges
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(
        edges,
        vec![
            ("@acme/api", "@acme/db"),
            ("@acme/api", "@acme/shared"),
            ("@acme/web", "@acme/shared"),
        ]
    );
    assert!(l.cycles.is_empty());
    let p = RepoPath::new("apps/api/src/main.ts").unwrap();
    assert_eq!(l.package_for(&p).unwrap().name, "@acme/api");
}

#[test]
fn negated_glob_excludes_package() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"packages/*\"\n  - \"!packages/legacy\"\n",
    );
    pkg_json(tmp.path(), "packages/a", "{\"name\":\"a\"}");
    pkg_json(tmp.path(), "packages/legacy", "{\"name\":\"legacy\"}");
    let (l, _) = layout(tmp.path());
    assert_eq!(names(&l), vec!["a"]);
}

#[test]
fn npm_workspaces_array_and_object_forms() {
    let a = tempfile::tempdir().unwrap();
    write_file(
        a.path(),
        "package.json",
        "{\"name\":\"root\",\"workspaces\":[\"libs/*\"]}",
    );
    pkg_json(a.path(), "libs/x", "{\"name\":\"x\"}");
    pkg_json(a.path(), "libs/y", "{\"name\":\"y\"}");
    let (l, _) = layout(a.path());
    assert_eq!(names(&l), vec!["x", "y"]);
    assert_eq!(l.tools, vec![WorkspaceTool::NpmWorkspaces]);

    let b = tempfile::tempdir().unwrap();
    write_file(
        b.path(),
        "package.json",
        "{\"name\":\"root\",\"workspaces\":{\"packages\":[\"libs/**\"]}}",
    );
    write_file(b.path(), "yarn.lock", "# yarn lockfile v1\n");
    pkg_json(b.path(), "libs/deep/x", "{\"name\":\"x\"}");
    write_file(b.path(), "libs/deep/x/src/a.ts", "export {};\n");
    let (l, warnings) = layout(b.path());
    assert_eq!(names(&l), vec!["x"]);
    assert_eq!(l.tools, vec![WorkspaceTool::Yarn]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!l.is_monorepo);
}

#[test]
fn nx_project_json_packages() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "nx.json",
        "{\"workspaceLayout\":{\"appsDir\":\"apps\",\"libsDir\":\"libs\"}}",
    );
    write_file(tmp.path(), "apps/shop/project.json", "{\"name\":\"shop\"}");
    write_file(tmp.path(), "libs/ui/project.json", "{\"name\":\"ui\"}");
    let (l, _) = layout(tmp.path());
    assert_eq!(names(&l), vec!["shop", "ui"]);
    assert_eq!(l.nx_apps_dir.as_deref(), Some("apps"));
    assert_eq!(l.nx_libs_dir.as_deref(), Some("libs"));
    assert!(l.tools.contains(&WorkspaceTool::Nx));
}

#[test]
fn turbo_recorded_without_packages() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "turbo.json", "{\"pipeline\":{\"build\":{}}}");
    let (l, _) = layout(tmp.path());
    assert!(l.packages.is_empty());
    assert_eq!(l.tools, vec![WorkspaceTool::Turbo]);
    assert_eq!(l.turbo_tasks, vec!["build"]);
}

#[test]
fn cargo_workspace_members_and_exclude() {
    let repo = fixture_repo("polyglot-manifests");
    let (l, _) = layout(&repo);
    let cargo: Vec<&str> = l
        .packages
        .iter()
        .filter(|p| p.source == WorkspaceTool::Cargo)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(cargo, vec!["crate-a"]);

    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/skipped\"]\n",
    );
    for c in ["a", "b", "skipped"] {
        write_file(
            tmp.path(),
            &format!("crates/{c}/Cargo.toml"),
            format!("[package]\nname = \"{c}\"\nversion = \"0.1.0\"\n"),
        );
    }
    let (l, _) = layout(tmp.path());
    assert_eq!(names(&l), vec!["a", "b"]);
}

#[test]
fn go_work_use_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "go.work",
        "go 1.22\n\nuse (\n\t./svc\n\t./lib\n)\nuse ./tools\n",
    );
    for d in ["svc", "lib", "tools"] {
        write_file(
            tmp.path(),
            &format!("{d}/go.mod"),
            format!("module example.com/{d}\n"),
        );
    }
    let (l, _) = layout(tmp.path());
    assert_eq!(
        names(&l),
        vec!["example.com/lib", "example.com/svc", "example.com/tools"]
    );
    assert_eq!(l.tools, vec![WorkspaceTool::GoWork]);
}

#[test]
fn cycle_detected_and_warned() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"p/*\"\n",
    );
    pkg_json(
        tmp.path(),
        "p/a",
        "{\"name\":\"a\",\"dependencies\":{\"b\":\"workspace:*\"}}",
    );
    pkg_json(
        tmp.path(),
        "p/b",
        "{\"name\":\"b\",\"dependencies\":{\"c\":\"1.0.0\"}}",
    );
    pkg_json(
        tmp.path(),
        "p/c",
        "{\"name\":\"c\",\"dependencies\":{\"a\":\"workspace:*\"}}",
    );
    let (l, warnings) = layout(tmp.path());
    assert_eq!(
        l.cycles,
        vec![vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]]
    );
    assert!(warnings.iter().any(|w| w.code == "workspace_cycle"));
}

#[test]
fn package_for_longest_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"a\"\n  - \"a/inner\"\n",
    );
    pkg_json(tmp.path(), "a", "{\"name\":\"outer\"}");
    pkg_json(tmp.path(), "a/inner", "{\"name\":\"inner\"}");
    let (l, _) = layout(tmp.path());
    let p = |s: &str| RepoPath::new(s).unwrap();
    assert_eq!(l.package_for(&p("a/inner/x.ts")).unwrap().name, "inner");
    assert_eq!(l.package_for(&p("a/other/x.ts")).unwrap().name, "outer");
    assert!(l.package_for(&p("elsewhere/x.ts")).is_none());
}

#[test]
fn single_package_repo_is_not_monorepo() {
    let repo = fixture_repo("init-basic");
    let (l, _) = layout(&repo);
    assert!(!l.is_monorepo);
    assert!(l.packages.is_empty());
}

#[test]
fn dir_without_manifest_warned() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"packages/*\"\n",
    );
    pkg_json(tmp.path(), "packages/a", "{\"name\":\"a\"}");
    write_file(tmp.path(), "packages/empty/readme.md", "x\n");
    let (l, warnings) = layout(tmp.path());
    assert_eq!(names(&l), vec!["a"]);
    assert!(warnings
        .iter()
        .any(|w| w.code == "workspace_dir_without_manifest"));
}

#[test]
fn unnamed_and_duplicate_packages_warned() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"p/*\"\n",
    );
    pkg_json(tmp.path(), "p/a", "{\"name\":\"same\"}");
    pkg_json(tmp.path(), "p/b", "{\"name\":\"same\"}");
    pkg_json(tmp.path(), "p/c", "{}");
    let (l, warnings) = layout(tmp.path());
    assert_eq!(names(&l), vec!["same", "same", "c"]);
    assert!(warnings
        .iter()
        .any(|w| w.code == "duplicate_workspace_package_name"));
    assert!(warnings
        .iter()
        .any(|w| w.code == "workspace_package_unnamed"));
}
