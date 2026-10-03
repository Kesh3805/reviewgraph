#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use repository::facts::RepositoryFacts;
use repository::init::{run, InitOptions, InitOutcome};
use repository::review_dir::ReviewLock;
use repository::InitError;
use review_test_support::{edge_case_tree, fixture_copy_named, git, write_file, CANARIES};

fn opts(root: &Path) -> InitOptions {
    InitOptions::new(root)
}

fn is_written(outcome: &InitOutcome) -> bool {
    matches!(outcome, InitOutcome::Written { .. })
}

#[test]
fn init_creates_full_review_layout() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    let outcome = run(&opts(&root)).unwrap();
    assert!(is_written(&outcome));
    for rel in [
        ".review/repository.json",
        ".review/.gitignore",
        ".review/config.yaml",
        ".review/.lock",
        ".review/graph/nodes",
        ".review/graph/edges",
        ".review/graph/indexes",
        ".review/graph/metadata",
        ".review/graph/snapshots",
        ".review/ast",
        ".review/symbols",
        ".review/semantic",
        ".review/profile",
        ".review/snapshots",
        ".review/history",
        ".review/cache",
    ] {
        assert!(root.join(rel).exists(), "{rel} is missing");
    }
    assert!(root.join(".review/cache/generated.json").exists());
    let facts = outcome.facts();
    assert_eq!(facts.schema_version, 1);
    assert_eq!(facts.root_name, "init-basic");
}

#[test]
fn review_gitignore_ignores_all_but_config() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    run(&opts(&root)).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".review/.gitignore")).unwrap(),
        "*\n!.gitignore\n!config.yaml\n"
    );
    // Only config.yaml and .gitignore show up in git status.
    let status = git(&root, &["status", "--porcelain", "--untracked-files=all"]);
    let review_lines: Vec<&str> = status.lines().filter(|l| l.contains(".review")).collect();
    assert_eq!(review_lines.len(), 2, "{review_lines:?}");
    assert!(review_lines
        .iter()
        .any(|l| l.ends_with(".review/.gitignore")));
    assert!(review_lines
        .iter()
        .any(|l| l.ends_with(".review/config.yaml")));
}

#[test]
fn config_yaml_written_only_when_absent() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    run(&opts(&root)).unwrap();
    let config = root.join(".review/config.yaml");
    let original = fs::read_to_string(&config).unwrap();
    assert!(original.contains("version: 1"));
    fs::write(&config, "version: 1\nignore: []\n# user edit\n").unwrap();
    run(&InitOptions {
        force: true,
        ..opts(&root)
    })
    .unwrap();
    assert!(fs::read_to_string(&config).unwrap().contains("# user edit"));
}

#[test]
fn force_never_overwrites_config_yaml() {
    config_yaml_written_only_when_absent();
}

#[test]
fn second_run_is_up_to_date() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    let first = run(&opts(&root)).unwrap();
    assert!(is_written(&first));
    let second = run(&opts(&root)).unwrap();
    assert!(matches!(second, InitOutcome::UpToDate { .. }));
    assert_eq!(first.facts().facts_hash, second.facts().facts_hash);
}

#[test]
fn dirty_worktree_never_up_to_date() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    run(&opts(&root)).unwrap();
    write_file(&root, "src/new.ts", "export const n = 1;\n");
    let second = run(&opts(&root)).unwrap();
    assert!(is_written(&second));
    assert!(second.facts().git.as_ref().unwrap().dirty.is_dirty);
    let third = run(&opts(&root)).unwrap();
    assert!(is_written(&third), "a dirty tree is never up to date");
}

#[test]
fn force_recomputes_same_facts_hash() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    let first = run(&opts(&root)).unwrap();
    let forced = run(&InitOptions {
        force: true,
        ..opts(&root)
    })
    .unwrap();
    assert!(is_written(&forced));
    assert_eq!(first.facts().facts_hash, forced.facts().facts_hash);
}

#[test]
fn concurrent_init_second_gets_busy() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    run(&opts(&root)).unwrap();
    let held = ReviewLock::acquire(&root.join(".review")).unwrap();
    let err = run(&InitOptions {
        force: true,
        ..opts(&root)
    })
    .unwrap_err();
    assert!(matches!(err, InitError::Busy), "{err:?}");
    drop(held);
    assert!(run(&InitOptions {
        force: true,
        ..opts(&root)
    })
    .is_ok());
}

#[test]
fn atomic_write_leaves_old_file_on_failure() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    run(&opts(&root)).unwrap();
    let target = root.join(".review/repository.json");
    let before = fs::read(&target).unwrap();
    // Make the final rename fail: a directory cannot be replaced by a file.
    fs::remove_file(&target).unwrap();
    fs::create_dir(&target).unwrap();
    let err = run(&InitOptions {
        force: true,
        ..opts(&root)
    })
    .unwrap_err();
    assert!(matches!(err, InitError::Io { .. }), "{err:?}");
    assert!(target.is_dir(), "the existing entry is untouched");
    let strays: Vec<_> = fs::read_dir(root.join(".review"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".tmp"))
        .collect();
    assert!(strays.is_empty(), "temp files left behind: {strays:?}");
    // Restoring the file makes the next run succeed and match the earlier content shape.
    fs::remove_dir(&target).unwrap();
    fs::write(&target, &before).unwrap();
    assert!(run(&InitOptions {
        force: true,
        ..opts(&root)
    })
    .is_ok());
}

fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(a) => a.iter().for_each(|v| collect_strings(v, out)),
        serde_json::Value::Object(o) => o.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

#[test]
fn no_absolute_paths_in_output() {
    let tmp = edge_case_tree();
    let outcome = run(&InitOptions {
        allow_non_git: true,
        ..opts(tmp.path())
    })
    .unwrap();
    let value = serde_json::to_value(outcome.facts()).unwrap();
    let mut strings = Vec::new();
    collect_strings(&value, &mut strings);
    let host = tmp.path().to_string_lossy().into_owned();
    let drive = regex_like_drive();
    for s in &strings {
        assert!(!s.starts_with('/'), "absolute path-like string: {s}");
        assert!(!drive(s), "windows path-like string: {s}");
        assert!(!s.contains(&host), "host path leaked: {s}");
    }
}

fn regex_like_drive() -> impl Fn(&str) -> bool {
    |s: &str| {
        let b = s.as_bytes();
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
    }
}

#[test]
fn canaries_absent_from_repository_json() {
    let tmp = edge_case_tree();
    run(&InitOptions {
        allow_non_git: true,
        ..opts(tmp.path())
    })
    .unwrap();
    let json = fs::read_to_string(tmp.path().join(".review/repository.json")).unwrap();
    let cache = fs::read_to_string(tmp.path().join(".review/cache/generated.json")).unwrap();
    for canary in CANARIES {
        assert!(
            !json.contains(canary),
            "{canary} leaked into repository.json"
        );
        assert!(!cache.contains(canary), "{canary} leaked into the cache");
    }
    let facts: RepositoryFacts = serde_json::from_str(&json).unwrap();
    assert!(facts
        .sensitive_files
        .iter()
        .any(|p| p.as_str() == "gcs-key.json"));
}

#[test]
fn list_truncation_is_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let deps: Vec<String> = (0..2100)
        .map(|i| format!("\"dep-{i:04}\":\"1.0.0\""))
        .collect();
    write_file(
        tmp.path(),
        "package.json",
        format!(
            "{{\"name\":\"big\",\"dependencies\":{{{}}}}}",
            deps.join(",")
        ),
    );
    let names: String = (0..600).map(|i| format!("VAR_{i}=\n")).collect();
    write_file(tmp.path(), ".env.example", names);
    let outcome = run(&InitOptions {
        allow_non_git: true,
        ..opts(tmp.path())
    })
    .unwrap();
    let facts = outcome.facts();
    assert_eq!(facts.manifests[0].dependencies.len(), 2000);
    assert_eq!(facts.env_files[0].variable_names.len(), 500);
    let truncated = facts
        .warnings
        .iter()
        .filter(|w| w.code == "list_truncated")
        .count();
    assert_eq!(truncated, 2);
}

#[test]
fn computed_mode_writes_nothing() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    let outcome = run(&InitOptions {
        write_review_dir: false,
        ..opts(&root)
    })
    .unwrap();
    assert!(matches!(outcome, InitOutcome::Computed { .. }));
    assert!(!root.join(".review").exists());
}

#[test]
fn non_git_directory_needs_opt_in() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "a.ts", "export {};\n");
    let err = run(&opts(tmp.path())).unwrap_err();
    assert!(matches!(err, InitError::NotAGitRepository(_)));
}

#[test]
fn facts_hash_pinned_for_init_basic() {
    let (_tmp, root) = fixture_copy_named("init-basic");
    let outcome = run(&InitOptions {
        write_review_dir: false,
        ..opts(&root)
    })
    .unwrap();
    // Changing the schema or the field order changes this hash: update it deliberately,
    // together with a REPOSITORY_FACTS_SCHEMA bump when the shape changed incompatibly.
    assert_eq!(
        outcome.facts().facts_hash,
        PINNED_INIT_BASIC_HASH,
        "facts_hash changed: update PINNED_INIT_BASIC_HASH if intentional"
    );
}

const PINNED_INIT_BASIC_HASH: &str =
    "355fd483be3d530ee949ab5dcda9a5c888f93a18e9eb5ce66e7b50e24575b702";

fn redacted(facts: &RepositoryFacts) -> serde_json::Value {
    let mut value = serde_json::to_value(facts).unwrap();
    value["detected_at"] = "[redacted]".into();
    value["tool_version"] = "[redacted]".into();
    value["facts_hash"] = "[redacted]".into();
    value["root_name"] = "[redacted]".into();
    if let Some(head) = value.pointer_mut("/git/head") {
        head["sha"] = "[redacted]".into();
    }
    value
}

#[test]
fn repository_json_snapshot_monorepo_pnpm() {
    let (_tmp, root) = fixture_copy_named("monorepo-pnpm");
    let outcome = run(&InitOptions {
        write_review_dir: false,
        ..opts(&root)
    })
    .unwrap();
    insta::assert_json_snapshot!("repository_json__monorepo_pnpm", redacted(outcome.facts()));
}

#[test]
fn prd_13_coverage_fields_present() {
    let (_tmp, root) = fixture_copy_named("monorepo-pnpm");
    let facts = run(&InitOptions {
        write_review_dir: false,
        ..opts(&root)
    })
    .unwrap()
    .facts()
    .clone();
    assert!(!facts.languages.is_empty()); // 1 languages
    assert!(!facts.frameworks.is_empty()); // 2 frameworks, 5 test frameworks share this list
    assert!(!facts.package_managers.is_empty()); // 3
    assert!(!facts.build_systems.is_empty()); // 4
    assert!(!facts.layout.source_roots.is_empty()); // 6
    assert!(!facts.layout.test_globs.is_empty()); // 7
    assert!(facts.generated.config_globs.is_empty() || facts.generated.files > 0); // 8
    assert!(!facts.manifests.is_empty()); // 9
    assert!(facts.workspaces.is_monorepo); // 10
    assert!(!facts.entrypoints.is_empty()); // 11, 13, 14
    assert_eq!(facts.api_routes.status, "deferred_to_index"); // 12
    assert!(facts.migrations.is_empty() && facts.schema_files.is_empty()); // 15 (none in this repo)
    assert!(facts.infra.is_empty()); // 16
    assert!(facts.auth.libraries.is_empty()); // 17
    assert!(!facts.tooling.compiler.is_empty()); // 18, 19
    assert!(facts.tooling.ci.is_empty()); // 20
    assert!(facts.docs.adrs.is_empty()); // 21, 22
}

#[test]
fn schema_matches_committed_contract() {
    let fresh = serde_json::to_value(
        schemars::gen::SchemaGenerator::default().into_root_schema_for::<RepositoryFacts>(),
    )
    .unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../packages/contracts/schemas/RepositoryFacts.schema.json");
    let mut committed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|_| "null".to_owned()))
            .unwrap();
    if let Some(map) = committed.as_object_mut() {
        map.remove("$id");
    }
    assert_eq!(
        fresh, committed,
        "RepositoryFacts schema drifted: run `review contracts export` and commit packages/contracts"
    );
}
