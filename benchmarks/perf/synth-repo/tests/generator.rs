#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use synth_repo::{
    apply_churn, build, changed_paths, commit_base, commit_head, generate, Plan, Profile,
};

fn plan_s(seed: u64) -> Plan {
    Plan::new(Profile::S.spec(), seed)
}

#[test]
fn same_seed_same_tree_hash() {
    let a = build(&plan_s(7), None, 2).unwrap();
    let b = build(&plan_s(7), None, 2).unwrap();
    assert_eq!(a.tree_hash, b.tree_hash);
    assert_eq!(a, b);
}

#[test]
fn different_seed_different_tree_hash() {
    let a = build(&plan_s(7), None, 2).unwrap();
    let b = build(&plan_s(8), None, 2).unwrap();
    assert_ne!(a.tree_hash, b.tree_hash);
}

#[test]
fn thread_count_does_not_change_output() {
    let one = build(&plan_s(11), None, 1).unwrap();
    let many = build(&plan_s(11), None, 4).unwrap();
    assert_eq!(one, many);
}

#[test]
fn profile_s_counts_within_5pct_of_target() {
    let plan = plan_s(42);
    let manifest = build(&plan, None, 2).unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&manifest).unwrap());
    let within = |actual: u64, target: u64| {
        let (a, t) = (actual as f64, target as f64);
        (a - t).abs() / t <= 0.05
    };
    assert!(
        within(manifest.counts.files, plan.spec.target_files),
        "files {} vs {}",
        manifest.counts.files,
        plan.spec.target_files
    );
    assert!(
        within(manifest.symbols_total, plan.spec.target_symbols),
        "symbols {} vs {}",
        manifest.symbols_total,
        plan.spec.target_symbols
    );
}

#[test]
#[ignore = "generates 100k files; run with --release --ignored (bench workflow)"]
fn large_profile_targets_hold_without_writing() {
    // Dry run of profile l: 100k files (+-2%), 1M symbols (+-10%), >= 2M planted edges.
    let plan = Plan::new(Profile::L.spec(), 42);
    let manifest = build(&plan, None, 4).unwrap();
    eprintln!(
        "l: files={} symbols={} edges={} bytes={} hub_top={:?}",
        manifest.counts.files,
        manifest.symbols_total,
        manifest.edges_estimated,
        manifest.counts.bytes,
        manifest.hub_fanin_top
    );
    let files = manifest.counts.files as f64;
    assert!(
        (files - 100_000.0).abs() / 100_000.0 <= 0.02,
        "files {files}"
    );
    let symbols = manifest.symbols_total as f64;
    assert!(
        (symbols - 1_000_000.0).abs() / 1_000_000.0 <= 0.10,
        "symbols {symbols}"
    );
    assert!(manifest.edges_estimated >= 2_000_000);
    assert!(
        manifest.hub_fanin_top.first().copied().unwrap_or(0) > 10_000,
        "top hub fan-in {:?}",
        manifest.hub_fanin_top
    );
}

#[test]
fn hub_fanin_follows_zipf_bounds() {
    let manifest = build(&plan_s(42), None, 2).unwrap();
    let top = &manifest.hub_fanin_top;
    assert!(top.len() >= 5, "hubs used: {top:?}");
    assert!(top.windows(2).all(|w| w[0] >= w[1]));
    // Zipf(1): the heaviest hub carries several times the load of the fifth.
    assert!(top[0] >= 2 * top[4], "{top:?}");
}

#[test]
fn ambiguous_and_generated_files_are_planted() {
    let manifest = build(&plan_s(42), None, 2).unwrap();
    let c = &manifest.counts;
    let ratio = c.calls_ambiguous as f64 / c.calls_total() as f64;
    assert!(ratio > 0.0 && ratio < 0.05, "ambiguous ratio {ratio}");
    assert!(c.generated_files > 0);
    assert!(c.routes > 0 && c.entities > 0 && c.processors > 0);
}

#[test]
fn refuses_dirty_output_dir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("stray.txt"), "x").unwrap();
    let err = generate(&plan_s(1), dir.path(), 2).unwrap_err();
    assert!(err.to_string().contains("non-empty"), "{err:#}");
}

#[test]
fn matching_output_dir_is_verified_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tree");
    let first = generate(&plan_s(3), &out, 2).unwrap();
    let second = generate(&plan_s(3), &out, 2).unwrap();
    assert_eq!(first, second);
    // A different seed over the same directory is refused.
    assert!(generate(&plan_s(4), &out, 2).is_err());
    // A tampered tree is refused too.
    let victim = walk_first_ts(&out);
    std::fs::write(&victim, "// edited\n").unwrap();
    assert!(generate(&plan_s(3), &out, 2).is_err());
}

fn walk_first_ts(dir: &std::path::Path) -> std::path::PathBuf {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let found = walk_first_ts(&path);
            if !found.as_os_str().is_empty() {
                return found;
            }
        } else if path.extension().is_some_and(|e| e == "ts") {
            return path;
        }
    }
    std::path::PathBuf::new()
}

#[test]
fn churn_modifies_exactly_n_files() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tree");
    let plan = plan_s(5);
    generate(&plan, &out, 2).unwrap();
    let base = commit_base(&out).unwrap();
    let (head, changed) = commit_head(&plan, &out, 7).unwrap();
    assert_eq!(changed.len(), 7);
    let paths = changed_paths(&out, &base, &head).unwrap();
    assert_eq!(paths.len(), 7, "{paths:?}");
    // SHAs are stable for a fixed seed.
    let dir2 = tempfile::tempdir().unwrap();
    let out2 = dir2.path().join("tree");
    generate(&plan, &out2, 1).unwrap();
    assert_eq!(base, commit_base(&out2).unwrap());
}

#[test]
fn bench_sample_selection_deterministic() {
    let plan = plan_s(42);
    let a = synth_repo::sample_files(&plan, 200, 1);
    let b = synth_repo::sample_files(&plan, 200, 1);
    let paths = |v: &[synth_repo::FileOut]| v.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    assert_eq!(a.len(), 200);
    assert_eq!(paths(&a), paths(&b));
    let other = synth_repo::sample_files(&plan, 200, 2);
    assert_ne!(paths(&a), paths(&other));
    for suffix in [".controller.ts", ".service.ts", ".entity.ts", ".spec.ts"] {
        assert!(a.iter().any(|f| f.path.ends_with(suffix)), "no {suffix}");
    }
}

#[test]
fn churn_larger_than_feature_count_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tree");
    let plan = plan_s(5);
    generate(&plan, &out, 2).unwrap();
    assert!(apply_churn(&plan, &out, 10_000).is_err());
}
