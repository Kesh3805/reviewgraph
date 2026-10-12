//! INV-011: the system can never merge. The TypeScript side is covered by
//! `apps/api/test/invariants/no-merge.spec.ts`; this scans all engine crate and app sources.

mod common;

use common::{engine_root, read_allowlist, rel, rust_files, TestResult};
use std::fs;

const FORBIDDEN: &[&str] = &[
    "pulls.merge",
    "/merge",
    "enablePullRequestAutoMerge",
    "mergePullRequest",
    "merge_method",
    "\"APPROVE\"",
    "REQUEST_CHANGES",
];

#[test]
fn inv_011_no_merge_identifiers_in_engine_source() -> TestResult {
    let allowed: Vec<String> = read_allowlist("merge-tokens.txt")?
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let mut dirs = Vec::new();
    for group in ["crates", "apps"] {
        for entry in fs::read_dir(engine_root().join(group))? {
            dirs.push(entry?.path().join("src"));
        }
    }
    let mut hits = Vec::new();
    for dir in dirs {
        for file in rust_files(&dir)? {
            let relative = rel(&file);
            if allowed.contains(&relative) {
                continue;
            }
            let text = fs::read_to_string(&file)?;
            for (idx, line) in text.lines().enumerate() {
                for token in FORBIDDEN {
                    if line.contains(token) {
                        hits.push(format!("{relative}:{}: {token}", idx + 1));
                    }
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "merge/approve capability in engine source: {hits:#?}"
    );
    Ok(())
}

#[test]
fn inv_011_merge_allowlist_entries_have_reasons() -> TestResult {
    for (path, reason) in read_allowlist("merge-tokens.txt")? {
        assert!(!reason.is_empty(), "allowlist entry {path} has no reason");
        assert!(engine_root().join(&path).is_file(), "{path} is missing");
    }
    Ok(())
}

#[test]
#[ignore = "blocked on E2E-001: the fake provider recorder does not exist yet"]
fn inv_011_e2e_recorder_sees_no_merge_request() {
    unimplemented!("E2E-001");
}
