#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

/// `PARSER_VERSIONS` is part of the repository fingerprint, so a dependency bump that forgets
/// the constant must fail the build.
#[test]
fn parser_versions_match_cargo_lock() {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let doc: toml::Table = std::fs::read_to_string(lock).unwrap().parse().unwrap();
    let packages = doc["package"].as_array().unwrap();
    for (name, version) in lang_typescript::PARSER_VERSIONS {
        let locked: Vec<&str> = packages
            .iter()
            .filter(|p| p["name"].as_str() == Some(name))
            .map(|p| p["version"].as_str().unwrap())
            .collect();
        assert_eq!(locked, vec![*version], "Cargo.lock disagrees about {name}");
    }
    let mut sorted = lang_typescript::PARSER_VERSIONS.to_vec();
    sorted.sort();
    assert_eq!(
        sorted,
        lang_typescript::PARSER_VERSIONS.to_vec(),
        "must be sorted by name"
    );
}
