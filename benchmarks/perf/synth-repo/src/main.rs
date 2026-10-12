//! `synth-repo --profile <s|m|l> --seed <u64> --out <dir> [--git] [--churn <n>]`

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Result};
use clap::Parser;
use synth_repo::{build, commit_base, commit_head, generate, Plan, Profile, GENERATOR_VERSION};

#[derive(Debug, Parser)]
#[command(
    name = "synth-repo",
    version,
    about = "Deterministic synthetic repository generator"
)]
struct Args {
    #[arg(long, value_parser = clap::value_parser!(Profile))]
    profile: Profile,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Output directory (not needed with --dry-run).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Commit the tree with a fixed author and date.
    #[arg(long)]
    git: bool,
    /// With --git: add a head commit that modifies N files.
    #[arg(long)]
    churn: Option<usize>,
    /// Worker threads (default: all cores). Output does not depend on it.
    #[arg(long)]
    threads: Option<usize>,
    /// Compute the manifest without writing files; prints it to stdout.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let plan = Plan::new(args.profile.spec(), args.seed);
    let threads = args
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    let started = Instant::now();
    if args.dry_run {
        let manifest = build(&plan, None, threads)?;
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        return Ok(());
    }
    let Some(out) = args.out else {
        bail!("--out is required unless --dry-run is set");
    };
    if args.churn.is_some() && !args.git {
        bail!("--churn requires --git");
    }
    let manifest = generate(&plan, &out, threads)?;
    let secs = started.elapsed().as_secs_f64().max(f64::EPSILON);
    eprintln!(
        "synth-repo v{GENERATOR_VERSION} profile={} seed={} files={} symbols={} edges~{} tree_hash={} ({:.1} files/s)",
        manifest.profile,
        manifest.seed,
        manifest.counts.files,
        manifest.symbols_total,
        manifest.edges_estimated,
        manifest.tree_hash,
        manifest.counts.files as f64 / secs
    );
    if args.git {
        let base = commit_base(&out)?;
        println!("base {base}");
        if let Some(n) = args.churn {
            let (head, changed) = commit_head(&plan, &out, n)?;
            println!("head {head}");
            eprintln!("churned {} files", changed.len());
        }
    }
    Ok(())
}
