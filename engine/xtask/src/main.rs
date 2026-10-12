//! `cargo run -p xtask -- <command>`: workspace maintenance commands.

use std::path::{Path, PathBuf};

fn engine_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn metrics_catalog(check: bool) -> anyhow::Result<()> {
    let rendered = xtask::metrics_catalog_markdown();
    let path = xtask::metrics_catalog_path(&engine_dir());
    if check {
        let committed = std::fs::read_to_string(&path)?.replace("\r\n", "\n");
        if committed != rendered {
            anyhow::bail!(
                "{} is stale; run `cargo run -p xtask -- metrics-catalog`",
                path.display()
            );
        }
        return Ok(());
    }
    std::fs::write(&path, rendered)?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("metrics-catalog") => metrics_catalog(args.iter().any(|a| a == "--check")),
        Some("check-deps") => {
            let violations = xtask::check(&engine_dir())?;
            if violations.is_empty() {
                Ok(())
            } else {
                anyhow::bail!("dependency violations: {violations:#?}")
            }
        }
        _ => anyhow::bail!("usage: xtask <metrics-catalog [--check] | check-deps>"),
    }
}
