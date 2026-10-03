#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use repository::build_systems::{detect_build_systems, BuildSystemKind};
use repository::manifests::{
    detect_manifests, DepKind, Ecosystem, ManifestFacts, PackageManagerKind, ParseStatusLite,
};
use repository::read::BoundedReader;
use repository::walk::{walk, WalkOptions};
use repository::InitWarning;
use review_test_support::{fixture_repo, write_file};

fn detect(root: &Path) -> (ManifestFacts, Vec<InitWarning>) {
    let (inv, _) = walk(root, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    detect_manifests(&inv, &reader)
}

fn pm_kinds(facts: &ManifestFacts, dir: &str) -> Vec<(PackageManagerKind, bool)> {
    facts
        .package_managers
        .iter()
        .filter(|p| p.scope_dir.as_str() == dir)
        .map(|p| (p.kind, p.primary))
        .collect()
}

#[test]
fn npm_lockfile_version_detected() {
    let repo = fixture_repo("polyglot-manifests");
    let (facts, _) = detect(&repo);
    let root = facts
        .package_managers
        .iter()
        .find(|p| p.scope_dir.is_root() && p.kind == PackageManagerKind::Npm)
        .unwrap();
    assert_eq!(root.lockfile_version.as_deref(), Some("3"));
    assert_eq!(root.declared.as_deref(), Some("npm@10.2.0"));
    assert!(root.primary);
}

#[test]
fn pnpm_lock_and_package_manager_field() {
    let repo = fixture_repo("monorepo-pnpm");
    let (facts, _) = detect(&repo);
    let pm = &facts.package_managers[0];
    assert_eq!(pm.kind, PackageManagerKind::Pnpm);
    assert_eq!(pm.lockfile_version.as_deref(), Some("9.0"));
    assert_eq!(pm.declared.as_deref(), Some("pnpm@10.4.1"));
    assert!(pm.primary);
}

#[test]
fn yarn_classic_vs_berry() {
    let repo = fixture_repo("polyglot-manifests");
    let (facts, _) = detect(&repo);
    assert_eq!(
        pm_kinds(&facts, "classic"),
        vec![(PackageManagerKind::YarnClassic, true)]
    );
    assert_eq!(
        pm_kinds(&facts, "berry"),
        vec![(PackageManagerKind::YarnBerry, true)]
    );
    let berry = facts
        .package_managers
        .iter()
        .find(|p| p.scope_dir.as_str() == "berry")
        .unwrap();
    assert_eq!(berry.lockfile_version.as_deref(), Some("8"));
}

#[test]
fn bun_text_and_binary_lock() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "a/package.json", "{\"name\":\"a\"}");
    write_file(tmp.path(), "a/bun.lock", "{\"lockfileVersion\": 1}");
    write_file(tmp.path(), "b/package.json", "{\"name\":\"b\"}");
    std::fs::write(tmp.path().join("b/bun.lockb"), [0u8, 1, 2]).unwrap();
    let (facts, _) = detect(tmp.path());
    assert_eq!(pm_kinds(&facts, "a"), vec![(PackageManagerKind::Bun, true)]);
    assert_eq!(pm_kinds(&facts, "b"), vec![(PackageManagerKind::Bun, true)]);
}

#[test]
fn multiple_lockfiles_warns_and_prefers_package_manager_field() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "package.json",
        "{\"name\":\"a\",\"packageManager\":\"npm@10.0.0\"}",
    );
    write_file(tmp.path(), "package-lock.json", "{\"lockfileVersion\": 3}");
    write_file(tmp.path(), "pnpm-lock.yaml", "lockfileVersion: '9.0'\n");
    let (facts, warnings) = detect(tmp.path());
    assert!(warnings.iter().any(|w| w.code == "multiple_lockfiles"));
    let primary: Vec<_> = facts
        .package_managers
        .iter()
        .filter(|p| p.primary)
        .collect();
    assert_eq!(primary.len(), 1);
    assert_eq!(primary[0].kind, PackageManagerKind::Npm);
    assert_eq!(facts.package_managers.len(), 2);

    // without a declared manager, pnpm wins by priority
    write_file(tmp.path(), "package.json", "{\"name\":\"a\"}");
    let (facts, _) = detect(tmp.path());
    let primary = facts.package_managers.iter().find(|p| p.primary).unwrap();
    assert_eq!(primary.kind, PackageManagerKind::Pnpm);
}

#[test]
fn package_json_dependencies_by_kind() {
    let repo = fixture_repo("init-basic");
    let (facts, _) = detect(&repo);
    let m = facts.manifest("package.json").unwrap();
    assert_eq!(m.name.as_deref(), Some("init-basic"));
    assert_eq!(m.dependencies["express"].kind, DepKind::Prod);
    assert_eq!(m.dependencies["typescript"].kind, DepKind::Dev);
    assert_eq!(m.dependencies["jest"].kind, DepKind::Dev);
    assert!(m.private);
    assert_eq!(m.parse_status, ParseStatusLite::Ok);
}

#[test]
fn workspace_protocol_flagged() {
    let repo = fixture_repo("monorepo-pnpm");
    let (facts, _) = detect(&repo);
    let api = facts.manifest("apps/api/package.json").unwrap();
    assert!(api.dependencies["@acme/shared"].workspace_protocol);
    assert!(api.dependencies["@acme/db"].workspace_protocol);
    assert!(!api.dependencies["@nestjs/core"].workspace_protocol);
}

#[test]
fn script_values_not_stored_only_entry_hints() {
    let repo = fixture_repo("monorepo-pnpm");
    let (facts, _) = detect(&repo);
    let api = facts.manifest("apps/api/package.json").unwrap();
    let extras = api.npm.as_ref().unwrap();
    assert_eq!(extras.script_names, vec!["build", "start", "start:prod"]);
    assert_eq!(extras.script_entry_hints["start"], vec!["dist/main.js"]);
    assert_eq!(extras.script_entry_hints["start:prod"], vec!["dist/main"]);
    let json = serde_json::to_string(&facts).unwrap();
    assert!(!json.contains("hunter2"));
    assert!(!json.contains("--require"));
    assert!(!json.contains("nest build"));
}

#[test]
fn malformed_package_json_is_partial_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "a/package.json",
        "{\"name\": 7, \"dependencies\": {\"x\": \"1\"}}",
    );
    write_file(tmp.path(), "b/package.json", "{ not json");
    let (facts, warnings) = detect(tmp.path());
    let a = facts.manifest("a/package.json").unwrap();
    assert_eq!(a.parse_status, ParseStatusLite::Partial);
    assert!(a.dependencies.contains_key("x"));
    assert!(matches!(
        facts.manifest("b/package.json").unwrap().parse_status,
        ParseStatusLite::Failed { .. }
    ));
    assert!(warnings.iter().any(|w| w.code == "manifest_field_type"));
    assert!(warnings.iter().any(|w| w.code == "manifest_parse"));
}

#[test]
fn pyproject_pep621_and_poetry() {
    let repo = fixture_repo("polyglot-manifests");
    let (facts, _) = detect(&repo);
    let m = facts.manifest("pyproject.toml").unwrap();
    assert_eq!(m.name.as_deref(), Some("polyglot-py"));
    assert!(m.dependencies.contains_key("requests"));
    assert!(m.dependencies.contains_key("pydantic"));
    assert_eq!(m.dependencies["pytest"].kind, DepKind::Optional);
    let req = facts.manifest("requirements.txt").unwrap();
    assert!(req.dependencies.contains_key("flask"));
    assert!(req.dependencies.contains_key("numpy"));
    assert_eq!(req.dependencies.len(), 2);

    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "pyproject.toml",
        "[tool.poetry]\nname = \"p\"\nversion = \"1.0\"\n[tool.poetry.dependencies]\npython = \"^3.11\"\nfastapi = \"^0.110\"\n",
    );
    let (facts, _) = detect(tmp.path());
    assert_eq!(
        pm_kinds(&facts, ""),
        vec![(PackageManagerKind::Poetry, true)]
    );
    assert!(facts.manifests[0].dependencies.contains_key("fastapi"));
    assert!(!facts.manifests[0].dependencies.contains_key("python"));
}

#[test]
fn go_mod_module_path() {
    let repo = fixture_repo("polyglot-manifests");
    let (facts, _) = detect(&repo);
    let m = facts.manifest("go.mod").unwrap();
    assert_eq!(m.name.as_deref(), Some("example.com/polyglot"));
    assert_eq!(m.meta["go"], "1.22");
    assert_eq!(m.meta["require_count"], "3");
    assert_eq!(m.dependencies["github.com/gin-gonic/gin"].range, "v1.9.1");
}

#[test]
fn cargo_workspace_recorded() {
    let repo = fixture_repo("polyglot-manifests");
    let (facts, _) = detect(&repo);
    let root = facts.manifest("Cargo.toml").unwrap();
    assert_eq!(root.meta["workspace"], "true");
    assert_eq!(root.members, vec!["crates/*"]);
    let a = facts.manifest("crates/a/Cargo.toml").unwrap();
    assert_eq!(a.name.as_deref(), Some("crate-a"));
    assert_eq!(a.dependencies["anyhow"].range, "1");
}

#[test]
fn pom_modules_streamed() {
    let repo = fixture_repo("polyglot-manifests");
    let (facts, _) = detect(&repo);
    let m = facts.manifest("pom.xml").unwrap();
    assert_eq!(m.name.as_deref(), Some("org.parent:polyglot-java"));
    assert_eq!(m.version.as_deref(), Some("1.4.0"));
    assert_eq!(m.members, vec!["core", "web"]);
    assert_eq!(m.dependencies["junit:junit"].kind, DepKind::Dev);
    assert_eq!(
        m.dependencies["org.springframework:spring-core"].range,
        "6.1.0"
    );
}

#[test]
fn gradle_settings_includes() {
    let repo = fixture_repo("polyglot-manifests");
    let (facts, _) = detect(&repo);
    let m = facts.manifest("gradle-app/settings.gradle.kts").unwrap();
    assert_eq!(m.name.as_deref(), Some("gradle-app"));
    assert_eq!(m.members, vec!["app", "lib", "extra"]);
    assert_eq!(m.ecosystem, Ecosystem::Gradle);
}

#[test]
fn build_systems_detected() {
    let repo = fixture_repo("monorepo-pnpm");
    let (inv, _) = walk(&repo, &WalkOptions::default()).unwrap();
    let kinds: Vec<_> = detect_build_systems(&inv).iter().map(|b| b.kind).collect();
    assert!(kinds.contains(&BuildSystemKind::Turbo));
    assert!(kinds.contains(&BuildSystemKind::NestCli));
    assert!(kinds.contains(&BuildSystemKind::Tsc));
}

#[test]
fn polyglot_languages_present() {
    use repository::language::language_stats;
    use review_core::language::Language;
    let repo = fixture_repo("polyglot-manifests");
    let (inv, _) = walk(&repo, &WalkOptions::default()).unwrap();
    let reader = BoundedReader::new(&inv.root);
    let (stats, _) = language_stats(&inv, &reader, &[Language::Typescript, Language::Javascript]);
    for (lang, analyzable) in [
        (Language::Typescript, true),
        (Language::Python, false),
        (Language::Go, false),
        (Language::Rust, false),
        (Language::Java, false),
        (Language::Kotlin, false),
    ] {
        let s = stats.iter().find(|s| s.language == lang).unwrap();
        assert_eq!(s.analyzable, analyzable, "{lang}");
    }
}

/// Needs RG_REFERENCE_REPO_PATH.
#[test]
#[ignore = "needs RG_REFERENCE_REPO_PATH"]
fn reference_manifests() {
    let root = std::env::var("RG_REFERENCE_REPO_PATH").unwrap();
    let (facts, _) = detect(Path::new(&root));
    assert_eq!(facts.package_managers[0].kind, PackageManagerKind::Npm);
    let m = facts.manifest("package.json").unwrap();
    for dep in ["@nestjs/core", "typeorm", "bullmq"] {
        assert!(m.dependencies.contains_key(dep), "{dep}");
    }
    assert!(!serde_json::to_string(&facts)
        .unwrap()
        .contains("node -r ts-node/register"));
}
