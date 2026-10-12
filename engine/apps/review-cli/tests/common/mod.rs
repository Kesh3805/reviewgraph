//! Shared helpers for the evaluation harness tests.
#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use review_cli::eval::case::{list_files, EvalPaths};

/// The repository root (contains `benchmarks/` and `fixtures/`).
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root")
}

pub fn paths() -> EvalPaths {
    EvalPaths::new(root())
}

/// Copies case `id` into `<tmp>/<new_id>/`, letting `edit` rewrite case.yaml.
pub fn copy_case(tmp: &Path, id: &str, new_id: &str, edit: impl FnOnce(String) -> String) -> PathBuf {
    let src = paths().cases().join(id);
    let dst = tmp.join(new_id);
    for rel in list_files(&src).unwrap() {
        let to = dst.join(&rel);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(src.join(&rel), &to).unwrap();
    }
    let yaml = fs::read_to_string(dst.join("case.yaml")).unwrap();
    let yaml = yaml.replace(&format!("id: \"{id}\""), &format!("id: \"{new_id}\""));
    fs::write(dst.join("case.yaml"), edit(yaml)).unwrap();
    dst
}
