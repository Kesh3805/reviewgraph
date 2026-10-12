#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! EVAL-001: case format, loader, validator, deterministic builds and the corpus hash.

mod common;

use review_cli::eval::build::{build_into, parse_expected_shas};
use review_cli::eval::case::{corpus_hash, load_corpus, CaseValidator};

use common::{copy_case, paths};

#[test]
fn case_schema_accepts_example() {
    let v = CaseValidator::new(&paths()).unwrap();
    let case = v
        .load(&paths().cases().join("example-000-template"))
        .expect("example case is valid");
    assert!(case.case.is_example());
    assert!(case.context.is_some());
}

#[test]
fn case_rejects_unknown_class() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = copy_case(tmp.path(), "corr-002-pagination-off-by-one", "corr-902-copy", |y| {
        y.replace("classes: [\"correctness_regression\"]", "classes: [\"style\"]")
    });
    let errors = CaseValidator::new(&paths()).unwrap().load(&dir).unwrap_err();
    assert!(errors.iter().any(|e| e.contains("classes")), "{errors:?}");
}

#[test]
fn case_rejects_bad_symbol_id() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = copy_case(tmp.path(), "corr-002-pagination-off-by-one", "corr-903-copy", |y| {
        y.replace(
            "symbol: \"ts:src/orders/orders.repository#OrdersRepository.listForOrganization/method\"",
            "symbol: \"src/orders/orders.repository.ts:22\"",
        )
    });
    let errors = CaseValidator::new(&paths()).unwrap().load(&dir).unwrap_err();
    assert!(errors.iter().any(|e| e.contains("bad symbol id")), "{errors:?}");
}

#[test]
fn case_rejects_missing_location_path() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = copy_case(tmp.path(), "corr-003-missing-await", "corr-904-copy", |y| {
        y.replace(
            "path: \"src/ledger/ledger.service.ts\"",
            "path: \"src/ledger/missing.service.ts\"",
        )
    });
    let errors = CaseValidator::new(&paths()).unwrap().load(&dir).unwrap_err();
    assert!(errors.iter().any(|e| e.contains("not in the head or base tree")), "{errors:?}");
}

#[test]
fn safe_change_requires_no_expected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = copy_case(tmp.path(), "corr-002-pagination-off-by-one", "corr-905-copy", |y| {
        y.replace("classes: [\"correctness_regression\"]", "classes: [\"safe_change\"]")
            .replace(
                "expect_no_published_findings: false",
                "expect_no_published_findings: true",
            )
    });
    let errors = CaseValidator::new(&paths()).unwrap().load(&dir).unwrap_err();
    assert!(
        errors.iter().any(|e| e.contains("requires no expected findings")),
        "{errors:?}"
    );
}

#[test]
fn build_is_deterministic_sha() {
    let p = paths();
    let (cases, errors) = load_corpus(&p, Some("example-000-template")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let tmp = tempfile::tempdir().unwrap();
    let a = build_into(&p, &cases[0], &tmp.path().join("a")).unwrap();
    let b = build_into(&p, &cases[0], &tmp.path().join("b")).unwrap();
    assert_eq!((a.base_sha.clone(), a.head_sha.clone()), (b.base_sha, b.head_sha));
    assert_ne!(a.base_sha, a.head_sha);
}

/// Acceptance: builds on any machine match the committed SHAs.
#[test]
fn builds_match_expected_shas() {
    let p = paths();
    let expected =
        parse_expected_shas(&std::fs::read_to_string(p.quality().join("expected_shas.txt")).unwrap());
    let (cases, errors) = load_corpus(&p, None).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(expected.len(), cases.len());
    let tmp = tempfile::tempdir().unwrap();
    for (id, base, head) in expected {
        let case = cases.iter().find(|c| c.case.id == id).expect("case listed");
        let built = build_into(&p, case, &tmp.path().join(&id)).unwrap();
        assert_eq!((built.base_sha, built.head_sha), (base, head), "{id}");
    }
}

#[test]
fn corpus_hash_changes_when_case_changes() {
    let p = paths();
    let v = CaseValidator::new(&p).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dir = copy_case(tmp.path(), "corr-002-pagination-off-by-one", "corr-906-copy", |y| y);
    let a = v.load(&dir).unwrap();
    let h1 = corpus_hash(&p, std::slice::from_ref(&a)).unwrap();
    let yaml = std::fs::read_to_string(dir.join("case.yaml")).unwrap();
    std::fs::write(dir.join("case.yaml"), yaml.replace("version: 1", "version: 2")).unwrap();
    let b = v.load(&dir).unwrap();
    let h2 = corpus_hash(&p, &[b]).unwrap();
    assert_ne!(h1, h2);
}
