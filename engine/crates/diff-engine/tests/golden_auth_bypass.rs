#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::single_range_in_vec_init
)]

//! DIFF-007 golden tests over `fixtures/pull-requests/auth-bypass` (PRD §151).

mod support;

use std::path::Path;

use diff_engine::anchor::reconcile::local_sets;
use diff_engine::hunks::LineKind;
use diff_engine::model::DiffModel;
use diff_engine::symbol_map::{map_hunks, HitScope, MapConfig, SymbolMap};
use diff_engine::testkit::Scenario;
use review_core::change::{FileChangeStatus, Hunk};
use review_core::location::DiffSide;
use serde_json::{json, Value};

const AUTHORIZE: &str = "ts:src/auth/auth.service#AuthService.authorize/method";

fn model_and_symbols() -> (Scenario, DiffModel, SymbolMap) {
    let (scenario, diff) = support::load("auth-bypass");
    let units = support::units_for(&scenario, &diff);
    let map = map_hunks(&diff, &units, &MapConfig::default());
    (scenario, diff, map)
}

fn diff_json(diff: &DiffModel, map: &SymbolMap) -> Value {
    let files: Vec<Value> = diff
        .files
        .iter()
        .map(|f| {
            json!({
                "path": f.file.path.as_str(),
                "old_path": f.file.old_path.as_ref().map(|p| p.as_str()),
                "status": f.file.status,
                "disposition": f.disposition,
                "additions": f.lines.map(|l| l.additions),
                "deletions": f.lines.map(|l| l.deletions),
                "hunks": f.file.hunks,
            })
        })
        .collect();
    let mut anchors = serde_json::Map::new();
    for f in &diff.files {
        let (right, left) = local_sets(f);
        anchors.insert(
            f.file.path.as_str().to_owned(),
            json!({
                "right": support::ranges_json(right.ranges()),
                "left": support::ranges_json(left.ranges()),
            }),
        );
    }
    let symbols: Vec<Value> = map
        .hits
        .iter()
        .map(|h| {
            json!({
                "symbol_id": h.symbol_id.as_str(),
                "path": h.path.as_str(),
                "side": h.side,
                "scope": h.scope,
                "ranges": support::ranges_json(&h.ranges),
                "ranges_old": support::ranges_json(&h.ranges_old),
                "whole_symbol": h.whole_symbol,
                "touches_code": h.touches_code,
            })
        })
        .collect();
    json!({
        "files": files,
        "anchors": Value::Object(anchors),
        "symbols": symbols,
        "unmapped": map.unmapped.len(),
        "coverage": diff.coverage,
    })
}

#[test]
fn golden_auth_bypass_diff_json() {
    let (scenario, diff, map) = model_and_symbols();
    support::assert_golden(
        scenario.dir.join("expected/diff.json"),
        &diff_json(&diff, &map),
    );
}

#[test]
fn auth_bypass_changed_files_are_exactly_two() {
    let (scenario, diff, _) = model_and_symbols();
    let paths: Vec<(&str, FileChangeStatus)> = diff
        .files
        .iter()
        .map(|f| (f.file.path.as_str(), f.file.status))
        .collect();
    assert_eq!(
        paths,
        vec![
            ("src/auth/auth.service.ts", FileChangeStatus::Modified),
            ("src/util/format.ts", FileChangeStatus::Modified),
        ]
    );
    let name_status = scenario.expected("name-status.txt").unwrap();
    let ours: Vec<String> = diff
        .files
        .iter()
        .map(|f| format!("M\t{}", f.file.path.as_str()))
        .collect();
    assert_eq!(name_status.lines().collect::<Vec<_>>(), ours);
    assert!(diff.coverage.is_empty());
}

#[test]
fn auth_bypass_authorize_hunk_old_and_new_ranges() {
    let (_, diff, _) = model_and_symbols();
    let file = diff.file("src/auth/auth.service.ts").unwrap();
    assert_eq!(
        file.file.hunks,
        vec![Hunk {
            old_start: 7,
            old_lines: 6,
            new_start: 7,
            new_lines: 6
        }]
    );
    let del: Vec<u32> = file.hunks[0]
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Del)
        .filter_map(|l| l.old_no)
        .collect();
    let add: Vec<u32> = file.hunks[0]
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Add)
        .filter_map(|l| l.new_no)
        .collect();
    assert_eq!((del, add), (vec![10], vec![10]));
}

#[test]
fn auth_bypass_format_ts_is_comment_only() {
    let (_, diff, map) = model_and_symbols();
    let file = diff.file("src/util/format.ts").unwrap();
    let changed: Vec<bool> = file
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .filter(|l| l.kind != LineKind::Context)
        .map(|l| l.trivia)
        .collect();
    assert_eq!(changed, vec![true]);
    let hits: Vec<_> = map.hits_in("src/util/format.ts").collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].symbol_id.as_str(),
        "ts:src/util/format#formatName/function"
    );
    assert!(!hits[0].touches_code);
    // The decoy `authorizeHeader` is never touched.
    assert!(map
        .hits
        .iter()
        .all(|h| !h.symbol_id.as_str().contains("authorizeHeader")));
}

/// RIGHT and LEFT line sets from a `git diff -U3` hunk section.
fn git_sets(patch: &str, file: &str) -> (Vec<u32>, Vec<u32>) {
    let mut right = Vec::new();
    let mut left = Vec::new();
    let mut in_file = false;
    let (mut o, mut n) = (0u32, 0u32);
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            in_file = rest.ends_with(&format!("b/{file}"));
            continue;
        }
        if !in_file {
            continue;
        }
        if let Some(header) = line.strip_prefix("@@ -") {
            let ranges = &header[..header.find(" @@").unwrap()];
            let (old, new) = ranges.split_once(" +").unwrap();
            o = old.split(',').next().unwrap().parse().unwrap();
            n = new.split(',').next().unwrap().parse().unwrap();
            continue;
        }
        match line.as_bytes().first() {
            Some(b' ') => {
                left.push(o);
                right.push(n);
                o += 1;
                n += 1;
            }
            Some(b'-') if !line.starts_with("---") => {
                left.push(o);
                o += 1;
            }
            Some(b'+') if !line.starts_with("+++") => {
                right.push(n);
                n += 1;
            }
            _ => {}
        }
    }
    (right, left)
}

#[test]
fn auth_bypass_anchor_sets_match_git_u3() {
    let (scenario, diff, _) = model_and_symbols();
    let patch = scenario.expected("git-diff-u3.patch").unwrap();
    for file in &diff.files {
        let (right, left) = local_sets(file);
        let (git_right, git_left) = git_sets(&patch, file.file.path.as_str());
        assert_eq!(
            right.iter().collect::<Vec<_>>(),
            git_right,
            "RIGHT {}",
            file.file.path.as_str()
        );
        assert_eq!(
            left.iter().collect::<Vec<_>>(),
            git_left,
            "LEFT {}",
            file.file.path.as_str()
        );
    }
}

#[test]
fn auth_bypass_symbol_hit_is_authorize_method() {
    let (_, _, map) = model_and_symbols();
    let code_hits: Vec<_> = map.hits.iter().filter(|h| h.touches_code).collect();
    assert_eq!(code_hits.len(), 1, "{code_hits:?}");
    let hit = code_hits[0];
    assert_eq!(hit.symbol_id.as_str(), AUTHORIZE);
    assert_eq!(hit.side, DiffSide::Head);
    assert_eq!(hit.scope, HitScope::Body);
    assert!(!hit.whole_symbol);
    for decoy in [
        "ReportService",
        "PermissionService",
        "AdminService",
        "authorizeHeader",
    ] {
        assert!(map
            .hits
            .iter()
            .all(|h| !h.symbol_id.as_str().contains(decoy)));
    }
}

#[test]
fn fixture_build_is_reproducible() {
    let a = Scenario::load("auth-bypass").unwrap();
    let b = Scenario::load("auth-bypass").unwrap();
    assert_eq!(a.base, b.base);
    assert_eq!(a.head, b.head);
    assert_ne!(a.base, a.head);
    assert_eq!(a.head_files, b.head_files);
}

/// Patterns that must never appear in fixture trees (secrets and credentials).
const FORBIDDEN: [&str; 8] = [
    "AKIA",
    "-----BEGIN",
    "ghp_",
    "github_pat_",
    "xoxb-",
    "sk_live_",
    "password=",
    "api_key=",
];

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            scan(&path, hits);
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            for pattern in FORBIDDEN {
                if text.contains(pattern) {
                    hits.push(format!("{}: {pattern}", path.display()));
                }
            }
        }
    }
}

#[test]
fn fixture_tree_has_no_forbidden_strings() {
    let root = diff_engine::testkit::scenario::pull_requests_dir();
    let mut hits = Vec::new();
    scan(&root, &mut hits);
    assert!(hits.is_empty(), "{hits:?}");
}
