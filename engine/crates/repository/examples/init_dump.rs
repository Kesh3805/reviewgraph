//! Prints the `RepositoryFacts` JSON for a repository until the `review init` CLI exists.
//!
//!   cargo run -p repository --example init_dump -- <path> [--no-write] [--force] [--allow-non-git]

use repository::init::{run, InitOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut path = None;
    let mut opts_force = false;
    let mut no_write = false;
    let mut allow_non_git = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--no-write" => no_write = true,
            "--force" => opts_force = true,
            "--allow-non-git" => allow_non_git = true,
            other => path = Some(other.to_owned()),
        }
    }
    let path = path.ok_or("usage: init_dump <path> [--no-write] [--force] [--allow-non-git]")?;
    let mut opts = InitOptions::new(path);
    opts.force = opts_force;
    opts.allow_non_git = allow_non_git;
    opts.write_review_dir = !no_write;
    let outcome = run(&opts)?;
    println!("{}", outcome.facts().to_pretty_json()?);
    Ok(())
}
