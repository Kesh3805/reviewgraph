#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-005 static guarantees. The compile-fail checks themselves are doctests on the crate root
//! (`compile_fail` blocks in `src/lib.rs`, run by `cargo test`); these tests make sure the
//! guarantees and those doctests stay in place.

const LIB: &str = include_str!("../src/lib.rs");

#[test]
fn compile_fail_raw_client_access() {
    assert!(
        LIB.contains("\nmod qdrant;"),
        "the qdrant module must stay private"
    );
    assert!(!LIB.contains("pub mod qdrant"));
    assert!(
        !LIB.lines()
            .any(|l| !l.starts_with("//") && l.contains("QdrantClient")),
        "the raw client must not be re-exported"
    );
    assert!(LIB.contains("// compile_fail_raw_client_access"));
}

#[test]
fn compile_fail_search_without_scope() {
    assert!(LIB.contains("// compile_fail_search_without_scope"));
    let index = include_str!("../src/index.rs");
    let search = index
        .split("pub async fn search(")
        .nth(1)
        .expect("SemanticIndex::search");
    let params = search.split(')').next().unwrap();
    assert!(params.contains("scope: &TenantScope"), "{params}");
}

#[test]
fn raw_probe_not_present_in_release_build() {
    // The audit layer (and its request recorder) only exists with the `audit` feature, which only
    // dev-dependencies enable.
    assert!(LIB.contains("#[cfg(feature = \"audit\")]\npub mod audit;"));
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("[dev-dependencies]")
        .next()
        .unwrap();
    assert!(
        !deps.contains("audit"),
        "no normal dependency may enable the audit feature"
    );
}
