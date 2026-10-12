//! PROF-001: model round trip, file adapter conformance and schema export.

use profile::model::{json_schema, RepositoryProfile};
use profile::store::conformance::{run_conformance, sample_profile};
use profile::store::file::FileProfileStore;
use profile::store::ProfileStore;

#[test]
fn profile_roundtrip_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileProfileStore::new(dir.path());
    let p = sample_profile();
    assert!(store.put(&p).unwrap());
    let back: RepositoryProfile = store.get(&p.key()).unwrap().unwrap();
    assert_eq!(back, p);
}

#[test]
fn profile_insert_conflict_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileProfileStore::new(dir.path());
    let p = sample_profile();
    assert!(store.put(&p).unwrap());
    assert!(!store.put(&p).unwrap());
}

#[test]
fn profile_store_conformance_suite() {
    let dir = tempfile::tempdir().unwrap();
    run_conformance(&FileProfileStore::new(dir.path()));
}

#[test]
fn profile_schema_exported() {
    let v = serde_json::to_value(json_schema()).unwrap();
    assert!(v["definitions"]["Convention"].is_object());
}
