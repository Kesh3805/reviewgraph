#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

fn engine_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn workspace_respects_dependency_direction() {
    let violations = xtask::check(&engine_dir()).unwrap();
    assert!(
        violations.is_empty(),
        "dependency violations: {violations:#?}"
    );
}

#[test]
fn every_library_crate_is_declared() {
    let ws = xtask::read_workspace(&engine_dir()).unwrap();
    for (name, info) in ws {
        if !info.is_app && !xtask::COMPOSITION_ROOTS.contains(&name.as_str()) {
            assert!(
                xtask::ALLOWED.iter().any(|(k, _)| *k == name),
                "{name} must be listed in xtask::ALLOWED"
            );
        }
    }
}

#[test]
fn no_library_depends_on_anyhow() {
    let violations = xtask::check(&engine_dir()).unwrap();
    let offenders: Vec<_> = violations
        .iter()
        .filter(|v| v.dependency == "anyhow")
        .collect();
    assert!(
        offenders.is_empty(),
        "libraries depending on anyhow: {offenders:#?}"
    );
}

#[test]
fn anyhow_rule_flags_libraries_but_not_apps() {
    let tmp = tempdir();
    let mk = |group: &str, name: &str, deps: &str| {
        let p = tmp.join(group).join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("Cargo.toml"),
            format!(
                "[package]
name = \"{name}\"
[dependencies]
{deps}
"
            ),
        )
        .unwrap();
    };
    mk("crates", "review-core", "anyhow = \"1\"");
    mk("apps", "review-cli", "anyhow = \"1\"");
    let v = xtask::check(&tmp).unwrap();
    std::fs::remove_dir_all(&tmp).unwrap();
    assert!(v
        .iter()
        .any(|v| v.krate == "review-core" && v.dependency == "anyhow"));
    assert!(!v.iter().any(|v| v.krate == "review-cli"));
}

#[test]
fn detects_forbidden_internal_edge_and_banned_external_dependency() {
    let tmp = tempdir();
    let mk = |group: &str, name: &str, deps: &str| {
        let p = tmp.join(group).join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n[dependencies]\n{deps}\n"),
        )
        .unwrap();
    };
    mk("crates", "review-core", "codegraph = { path = \"x\" }");
    mk("crates", "codegraph", "review-cli = { path = \"y\" }");
    mk("crates", "reviewers", "reqwest = \"0.12\"");
    mk("apps", "review-cli", "codegraph = { path = \"x\" }");

    let v = xtask::check(&tmp).unwrap();
    std::fs::remove_dir_all(&tmp).unwrap();

    assert!(v
        .iter()
        .any(|v| v.krate == "review-core" && v.dependency == "codegraph"));
    assert!(v
        .iter()
        .any(|v| v.krate == "codegraph" && v.dependency == "review-cli"));
    assert!(v
        .iter()
        .any(|v| v.krate == "reviewers" && v.dependency == "reqwest"));
    assert!(
        !v.iter().any(|v| v.krate == "review-cli"),
        "apps may depend on libraries"
    );
}

fn tempdir() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rg-xtask-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
