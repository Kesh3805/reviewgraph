#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use repository::read::BoundedReader;
use repository::tsconfig::{detect_tsconfigs, TsConfigSet, TsExtends};
use repository::walk::{walk, WalkOptions};
use repository::InitWarning;
use review_core::location::RepoPath;
use review_test_support::{fixture_repo, write_file};

fn load(root: &Path) -> (TsConfigSet, Vec<InitWarning>) {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    detect_tsconfigs(&inv, &reader)
}

fn rp(s: &str) -> RepoPath {
    RepoPath::new(s).unwrap()
}

#[test]
fn tsconfig_with_comments_and_trailing_commas_parses() {
    let repo = fixture_repo("monorepo-pnpm");
    let (set, warnings) = load(&repo);
    let base = set.get("tsconfig.base.json").unwrap();
    assert_eq!(base.effective.paths.len(), 3);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn extends_relative_chain_merges_compiler_options() {
    let repo = fixture_repo("monorepo-pnpm");
    let (set, _) = load(&repo);
    let api = set.get("apps/api/tsconfig.json").unwrap();
    assert_eq!(api.effective.target.as_deref(), Some("ES2022"));
    assert_eq!(api.effective.strict, Some(true));
    assert_eq!(api.effective.emit_decorator_metadata, Some(true));
    assert_eq!(
        api.effective.out_dir.as_ref().unwrap().as_str(),
        "apps/api/dist"
    );
    assert_eq!(
        api.effective.root_dir.as_ref().unwrap().as_str(),
        "apps/api/src"
    );
    assert_eq!(
        api.extends_chain,
        vec![TsExtends::Local {
            path: rp("tsconfig.base.json")
        }]
    );
    let build = set.get("apps/api/tsconfig.build.json").unwrap();
    assert_eq!(build.extends_chain.len(), 2);
    assert_eq!(build.effective.exclude, vec!["**/*.spec.ts"]);
    assert_eq!(build.effective.include, vec!["src/**/*"]);
}

#[test]
fn paths_replaced_not_merged() {
    let repo = fixture_repo("monorepo-pnpm");
    let (set, _) = load(&repo);
    let web = set.get("apps/web/tsconfig.json").unwrap();
    assert_eq!(web.effective.paths.len(), 1);
    assert_eq!(web.effective.paths[0].pattern, "@/*");
    assert_eq!(web.effective.jsx.as_deref(), Some("preserve"));
}

#[test]
fn paths_relative_to_defining_config() {
    let repo = fixture_repo("monorepo-pnpm");
    let (set, _) = load(&repo);
    // api inherits paths from the root base config, so they stay relative to the root.
    let api = set.get("apps/api/tsconfig.json").unwrap();
    assert_eq!(api.effective.paths.len(), 3);
    assert!(api.effective.paths_base.is_root());
    let web = set.get("apps/web/tsconfig.json").unwrap();
    assert_eq!(web.effective.paths_base.as_str(), "apps/web");
}

#[test]
fn base_url_inherited_from_parent() {
    let repo = fixture_repo("monorepo-pnpm");
    let (set, _) = load(&repo);
    let api = set.get("apps/api/tsconfig.json").unwrap();
    assert!(api.effective.base_url.as_ref().unwrap().is_root());
}

#[test]
fn extends_array_applied_left_to_right() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "a.json",
        r#"{"compilerOptions":{"target":"ES2015","module":"commonjs"}}"#,
    );
    write_file(
        tmp.path(),
        "b.json",
        r#"{"compilerOptions":{"target":"ES2022"}}"#,
    );
    write_file(tmp.path(), "tsconfig.json", r#"{"extends":["./a","./b"]}"#);
    let (set, _) = load(tmp.path());
    let c = set.get("tsconfig.json").unwrap();
    assert_eq!(c.effective.target.as_deref(), Some("ES2022"));
    assert_eq!(c.effective.module.as_deref(), Some("commonjs"));
}

#[test]
fn extends_package_unresolved_warns() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "tsconfig.json",
        r#"{"extends":"@tsconfig/node20/tsconfig.json"}"#,
    );
    let (set, warnings) = load(tmp.path());
    assert_eq!(
        set.configs[0].extends_chain,
        vec![TsExtends::Package {
            specifier: "@tsconfig/node20/tsconfig.json".to_owned(),
            resolved: None
        }]
    );
    assert!(warnings
        .iter()
        .any(|w| w.code == "tsconfig_base_unresolved"));
}

#[test]
fn extends_cycle_detected() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "tsconfig.json", r#"{"extends":"./other.json"}"#);
    write_file(tmp.path(), "other.json", r#"{"extends":"./tsconfig.json"}"#);
    let (set, warnings) = load(tmp.path());
    assert!(warnings.iter().any(|w| w.code == "tsconfig_extends_cycle"));
    assert!(set.get("tsconfig.json").is_some() || set.configs.is_empty());
}

fn owned_tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "tsconfig.json",
        r#"{"compilerOptions":{"strict":true},"exclude":["scripts"]}"#,
    );
    write_file(tmp.path(), "src/a.ts", "export {};\n");
    write_file(tmp.path(), "scripts/s.ts", "export {};\n");
    write_file(tmp.path(), "pkg/tsconfig.json", r#"{"include":["lib"]}"#);
    write_file(tmp.path(), "pkg/lib/b.ts", "export {};\n");
    write_file(tmp.path(), "pkg/other/c.ts", "export {};\n");
    write_file(
        tmp.path(),
        "pkg/tsconfig.build.json",
        r#"{"include":["other"]}"#,
    );
    tmp
}

#[test]
fn ownership_nearest_including_config_wins() {
    let tmp = owned_tree();
    let (set, _) = load(tmp.path());
    let owner = set.tsconfig_for(&rp("pkg/lib/b.ts")).unwrap();
    assert_eq!(owner.config.path.as_str(), "pkg/tsconfig.json");
    assert!(!owner.owned_by_fallback);
    let owner = set.tsconfig_for(&rp("src/a.ts")).unwrap();
    assert_eq!(owner.config.path.as_str(), "tsconfig.json");
}

#[test]
fn excluded_file_falls_back_flagged() {
    let tmp = owned_tree();
    let (set, _) = load(tmp.path());
    // pkg/other/c.ts is not included by pkg/tsconfig.json; the root config includes it.
    let owner = set.tsconfig_for(&rp("pkg/other/c.ts")).unwrap();
    assert_eq!(owner.config.path.as_str(), "tsconfig.json");
    assert!(!owner.owned_by_fallback);
    // scripts/ is excluded by the only config: nearest config, flagged.
    let owner = set.tsconfig_for(&rp("scripts/s.ts")).unwrap();
    assert_eq!(owner.config.path.as_str(), "tsconfig.json");
    assert!(owner.owned_by_fallback);
}

#[test]
fn build_config_never_chosen_by_ownership() {
    let tmp = owned_tree();
    let (set, _) = load(tmp.path());
    for file in ["pkg/other/c.ts", "pkg/lib/b.ts", "src/a.ts"] {
        let owner = set.tsconfig_for(&rp(file)).unwrap();
        assert!(!owner.config.path.as_str().contains("build"), "{file}");
    }
}

#[test]
fn default_include_all() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "tsconfig.json", "{}");
    let (set, _) = load(tmp.path());
    assert_eq!(set.configs[0].effective.include, vec!["**/*"]);
}

#[test]
fn malformed_tsconfig_is_failed_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "tsconfig.json", "{ nope");
    let (set, warnings) = load(tmp.path());
    assert_eq!(set.configs.len(), 1);
    assert!(warnings.iter().any(|w| w.code == "tsconfig_parse"));
}

#[test]
#[ignore = "needs RG_REFERENCE_REPO_PATH"]
fn reference_tsconfig() {
    let root = std::env::var("RG_REFERENCE_REPO_PATH").unwrap();
    let (set, _) = load(Path::new(&root));
    let owner = set
        .tsconfig_for(&rp("src/features/v1/account/account.controller.ts"))
        .unwrap();
    assert_eq!(owner.config.path.as_str(), "tsconfig.json");
    assert_eq!(owner.config.effective.paths.len(), 7);
    assert!(owner.config.effective.base_url.as_ref().unwrap().is_root());
    assert_eq!(owner.config.effective.emit_decorator_metadata, Some(true));
}
