//! INV-004 (PRD section 147, invariant 4): no provider code in the engine domain.
//!
//! Domain crates must not depend on provider SDKs or (for the pure-logic crates) any HTTP
//! client, and their sources must not mention provider names except for allowlisted files.

mod common;

use common::{engine_root, read_allowlist, scan_crates, TestResult};
use std::fs;

const DOMAIN_CRATES: &[&str] = &[
    "review-core",
    "codegraph",
    "analysis-ir",
    "lang-typescript",
    "incremental",
    "diff-engine",
    "impact",
    "semantic",
    "context-engine",
    "reviewers",
    "verification",
    "profile",
    "pipeline",
];

/// Pure-logic crates: no HTTP client of any kind.
const NO_HTTP_CRATES: &[&str] = &[
    "reviewers",
    "verification",
    "codegraph",
    "impact",
    "diff-engine",
    "review-core",
];

const PROVIDER_DEPS: &[&str] = &["octocrab", "octokit", "gitlab", "bitbucket", "github"];
const HTTP_DEPS: &[&str] = &[
    "reqwest",
    "hyper",
    "ureq",
    "isahc",
    "surf",
    "attohttpc",
    "curl",
];
const PROVIDER_TOKENS: &[&str] = &[
    "github",
    "gitlab",
    "bitbucket",
    "octokit",
    "api.github.com",
    "x-github",
    "refs/pull/",
];

/// Dependency names declared in `[dependencies]`-style sections of a manifest.
fn dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_deps = line.contains("dependencies");
            continue;
        }
        if in_deps && !line.is_empty() && !line.starts_with('#') {
            if let Some((name, _)) = line.split_once('=') {
                names.push(name.trim().split('.').next().unwrap_or("").to_owned());
            }
        }
    }
    names
}

fn manifest_of(name: &str) -> Result<String, std::io::Error> {
    fs::read_to_string(engine_root().join("crates").join(name).join("Cargo.toml"))
}

#[test]
fn inv_004_domain_crates_do_not_depend_on_provider_or_http_crates() -> TestResult {
    let mut violations = Vec::new();
    for name in DOMAIN_CRATES {
        for dep in dependency_names(&manifest_of(name)?) {
            let d = dep.to_lowercase();
            if PROVIDER_DEPS.iter().any(|p| d.contains(p)) {
                violations.push(format!("{name} depends on provider crate {dep}"));
            }
            if NO_HTTP_CRATES.contains(name) && HTTP_DEPS.iter().any(|h| d == *h) {
                violations.push(format!("{name} depends on HTTP client {dep}"));
            }
        }
    }
    assert!(violations.is_empty(), "{violations:#?}");
    Ok(())
}

#[test]
fn inv_004_domain_sources_contain_no_provider_tokens() -> TestResult {
    let allowed: Vec<String> = read_allowlist("provider-tokens.txt")?
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let hits = scan_crates(DOMAIN_CRATES, PROVIDER_TOKENS, &allowed)?;
    assert!(
        hits.is_empty(),
        "provider tokens in domain sources (add to allowlists/provider-tokens.txt with a reason only if justified): {hits:#?}"
    );
    Ok(())
}

#[test]
fn inv_004_allowlist_entries_have_reasons() -> TestResult {
    for (path, reason) in read_allowlist("provider-tokens.txt")? {
        assert!(!reason.is_empty(), "allowlist entry {path} has no reason");
        assert!(
            engine_root().join(&path).is_file(),
            "allowlist entry {path} points at a missing file"
        );
    }
    Ok(())
}
