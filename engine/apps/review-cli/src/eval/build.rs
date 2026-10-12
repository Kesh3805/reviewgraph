//! Materialises a case into a deterministic git repo by running `benchmarks/quality/build.sh`.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// `(base_sha, head_sha)` of a built case.
pub fn build_case(script: &Path, case_dir: &Path, out_dir: &Path) -> Result<(String, String)> {
    let output = Command::new("bash")
        .arg(script)
        .arg(case_dir)
        .arg(out_dir)
        .output()
        .context("running build.sh")?;
    if !output.status.success() {
        bail!(
            "build.sh failed for {}: {}",
            case_dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut it = stdout.split_whitespace();
    match (it.next(), it.next()) {
        (Some(b), Some(h)) => Ok((b.to_string(), h.to_string())),
        _ => bail!("build.sh printed no SHAs"),
    }
}
