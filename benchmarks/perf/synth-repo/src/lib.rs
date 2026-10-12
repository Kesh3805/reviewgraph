//! Deterministic synthetic NestJS-shaped repository generator (PERF-001).
//!
//! `(seed, profile, generator_version)` fully determines every byte of the tree. Modules are
//! generated independently on a bounded rayon pool and merged in module order, so the output
//! and the manifest do not depend on the thread count. Nothing here touches the network.

pub mod manifest;
pub mod profile;
pub mod rng;
pub mod templates;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use rayon::prelude::*;

pub use manifest::{Counts, Manifest, GENERATOR_VERSION};
pub use profile::{Profile, ProfileSpec};
pub use templates::{ident, FileOut, Plan};

use rng::Rng;
use templates::{generate_module, generate_shared};

/// Counts plus the `(path, blake3 hex)` list of one generated module.
type ModuleResult = Result<(Counts, Vec<(String, String)>)>;

pub const MANIFEST_FILE: &str = "manifest.json";

/// What `prepare_output` decided about the output directory.
#[derive(Debug)]
pub enum Prepared {
    /// Empty or missing: generate into it.
    Fresh,
    /// Already holds a matching, intact tree: nothing to do.
    Verified(Box<Manifest>),
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn write_atomic(root: &Path, rel: &str, content: &str) -> Result<()> {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = target.with_extension("tmp-synth");
    fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, &target).with_context(|| format!("rename into {}", target.display()))?;
    Ok(())
}

/// Generates the tree (writing it when `out` is set) and returns its manifest.
pub fn build(plan: &Plan, out: Option<&Path>, threads: usize) -> Result<Manifest> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()
        .context("build thread pool")?;
    let modules = plan.spec.modules;
    let results: Vec<ModuleResult> = pool.install(|| {
        (0..modules + 1)
            .into_par_iter()
            .map(|mi| {
                let generated = if mi == modules {
                    generate_shared(plan)
                } else {
                    generate_module(plan, mi)
                };
                let mut hashes = Vec::with_capacity(generated.files.len());
                for file in &generated.files {
                    hashes.push((file.path.clone(), blake3_hex(file.content.as_bytes())));
                    if let Some(root) = out {
                        write_atomic(root, &file.path, &file.content)?;
                    }
                }
                Ok((generated.counts, hashes))
            })
            .collect()
    });
    let mut counts = Counts::default();
    let mut entries = Vec::new();
    for result in results {
        let (module_counts, hashes) = result?;
        counts.merge(&module_counts);
        entries.extend(hashes);
    }
    Ok(Manifest::new(
        plan.seed,
        &plan.spec.name,
        counts,
        manifest::tree_hash(entries),
    ))
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            walk(&path, root, out)?;
        } else {
            let rel = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if rel == MANIFEST_FILE {
                continue;
            }
            out.push((rel, blake3_hex(&fs::read(&path)?)));
        }
    }
    Ok(())
}

/// Recomputes the tree hash of a directory on disk.
pub fn hash_tree_on_disk(root: &Path) -> Result<String> {
    let mut entries = Vec::new();
    walk(root, root, &mut entries)?;
    Ok(manifest::tree_hash(entries))
}

/// Refuses to write into a non-empty directory unless it holds a matching, intact manifest.
pub fn prepare_output(out: &Path, plan: &Plan) -> Result<Prepared> {
    if !out.exists() || fs::read_dir(out)?.next().is_none() {
        return Ok(Prepared::Fresh);
    }
    let manifest_path = out.join(MANIFEST_FILE);
    if !manifest_path.exists() {
        bail!(
            "refusing to write into non-empty directory {} (no {MANIFEST_FILE})",
            out.display()
        );
    }
    let existing: Manifest = serde_json::from_str(&fs::read_to_string(&manifest_path)?)
        .context("parse existing manifest")?;
    if !existing.same_key(plan.seed, &plan.spec.name) {
        bail!(
            "refusing to write into {}: it holds a different (seed, profile, generator_version)",
            out.display()
        );
    }
    let on_disk = hash_tree_on_disk(out)?;
    if on_disk != existing.tree_hash {
        bail!(
            "existing tree in {} does not match its manifest (tree_hash {on_disk} != {})",
            out.display(),
            existing.tree_hash
        );
    }
    Ok(Prepared::Verified(Box::new(existing)))
}

/// Fails fast when the volume cannot hold the estimated output (estimate x 1.2).
pub fn check_disk_space(out: &Path, plan: &Plan) -> Result<()> {
    let estimate = plan.spec.target_files * plan.spec.est_bytes_per_file;
    let needed = estimate + estimate / 5;
    let mut probe: PathBuf = out.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent.to_path_buf(),
            None => return Ok(()),
        }
    }
    let available = fs2::available_space(&probe).context("query free disk space")?;
    if available < needed {
        bail!(
            "not enough disk space under {}: need ~{needed} bytes, have {available}",
            probe.display()
        );
    }
    Ok(())
}

/// Generates into `out` (idempotent: a matching intact tree is verified and left alone).
pub fn generate(plan: &Plan, out: &Path, threads: usize) -> Result<Manifest> {
    match prepare_output(out, plan)? {
        Prepared::Verified(manifest) => Ok(*manifest),
        Prepared::Fresh => {
            check_disk_space(out, plan)?;
            fs::create_dir_all(out)?;
            let manifest = build(plan, Some(out), threads)?;
            fs::write(
                out.join(MANIFEST_FILE),
                serde_json::to_string_pretty(&manifest)? + "\n",
            )?;
            Ok(manifest)
        }
    }
}

fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "synth")
        .env("GIT_AUTHOR_EMAIL", "synth@example.invalid")
        .env("GIT_COMMITTER_NAME", "synth")
        .env("GIT_COMMITTER_EMAIL", "synth@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_SYSTEM", null_device())
        .output()
        .with_context(|| format!("run git {args:?}"))?;
    if !output.status.success() {
        bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Commits the generated `src/` tree with a fixed author and date, so SHAs are stable.
/// Returns the base commit SHA.
pub fn commit_base(out: &Path) -> Result<String> {
    run_git(out, &["init", "-q", "-b", "main"])?;
    run_git(out, &["add", "-A", "--", "src"])?;
    run_git(out, &["commit", "-q", "-m", "base"])?;
    run_git(out, &["rev-parse", "HEAD"])
}

fn global_index(plan: &Plan, mi: u32, fi: u32) -> u64 {
    u64::from(mi) * u64::from(plan.spec.features_per_module) + u64::from(fi)
}

/// Edit classes, cycled by index: 0 body-only, 1 hub signature change, 2 file rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditClass {
    Body,
    HubSignature,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Churned {
    pub path: String,
    pub class: EditClass,
}

/// Modifies exactly `n` service files on disk and returns what was changed.
pub fn apply_churn(plan: &Plan, root: &Path, n: usize) -> Result<Vec<Churned>> {
    let total = plan.total_features();
    if n as u64 > total {
        bail!(
            "--churn {n} exceeds the {total} features of profile {}",
            plan.spec.name
        );
    }
    let features = u64::from(plan.spec.features_per_module);
    let mut order: Vec<u64> = (0..total).collect();
    let mut rng = Rng::seeded(plan.seed, "churn", 0);
    for i in (1..order.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        order.swap(i, j);
    }
    let mut random_iter = order.into_iter();
    let mut hub_iter = plan
        .hubs()
        .iter()
        .map(|&(mi, fi)| global_index(plan, mi, fi))
        .collect::<Vec<_>>()
        .into_iter();
    let mut used: HashSet<u64> = HashSet::new();
    let mut changed = Vec::new();
    for i in 0..n {
        let class = match i % 3 {
            0 => EditClass::Body,
            1 => EditClass::HubSignature,
            _ => EditClass::Rename,
        };
        let mut pick = None;
        if class == EditClass::HubSignature {
            pick = hub_iter.by_ref().find(|g| !used.contains(g));
        }
        let global = match pick {
            Some(g) => g,
            None => random_iter
                .by_ref()
                .find(|g| !used.contains(g))
                .context("ran out of features to churn")?,
        };
        used.insert(global);
        let id = ident(
            plan.seed,
            (global / features) as u32,
            (global % features) as u32,
        );
        let rel = id.service_path();
        let path = root.join(&rel);
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        match class {
            EditClass::Body => {
                let edited = text.replacen("const limit = 100;", "const limit = 101;", 1);
                if edited == text {
                    bail!("body marker missing in {rel}");
                }
                fs::write(&path, edited)?;
            }
            EditClass::HubSignature => {
                let edited = text.replacen(
                    "async findOne(id: string)",
                    "async findOne(id: string, opts?: unknown)",
                    1,
                );
                if edited == text {
                    bail!("findOne signature missing in {rel}");
                }
                fs::write(&path, edited)?;
            }
            EditClass::Rename => {
                let renamed = format!("{}/{}.renamed.service.ts", id.dir, id.kebab);
                fs::rename(&path, root.join(&renamed))?;
                changed.push(Churned {
                    path: renamed,
                    class,
                });
                continue;
            }
        }
        changed.push(Churned { path: rel, class });
    }
    Ok(changed)
}

/// Applies churn and commits it as `head`. Returns the head commit SHA.
pub fn commit_head(plan: &Plan, out: &Path, n: usize) -> Result<(String, Vec<Churned>)> {
    let changed = apply_churn(plan, out, n)?;
    run_git(out, &["add", "-A", "--", "src"])?;
    run_git(out, &["commit", "-q", "-m", "head"])?;
    Ok((run_git(out, &["rev-parse", "HEAD"])?, changed))
}

/// Generates the whole tree in memory and picks `n` files by a seeded shuffle: the fixed sample
/// the parse benchmarks use (mixed controllers, services, entities, DTOs and specs).
pub fn sample_files(plan: &Plan, n: usize, sample_seed: u64) -> Vec<FileOut> {
    let mut all: Vec<FileOut> = Vec::new();
    for mi in 0..plan.spec.modules {
        all.extend(generate_module(plan, mi).files);
    }
    all.extend(generate_shared(plan).files);
    let mut rng = Rng::seeded(plan.seed, "sample", sample_seed);
    let take = n.min(all.len());
    for i in 0..take {
        let j = i + rng.below((all.len() - i) as u64) as usize;
        all.swap(i, j);
    }
    all.truncate(take);
    all.sort_by(|a, b| a.path.cmp(&b.path));
    all
}

/// Paths changed between two commits, with rename detection (a rename counts once).
pub fn changed_paths(out: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    let text = run_git(out, &["diff", "--name-only", "-M", base, head])?;
    Ok(text.lines().map(str::to_owned).collect())
}
