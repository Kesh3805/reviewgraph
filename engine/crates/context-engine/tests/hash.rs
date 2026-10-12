#![allow(clippy::unwrap_used, clippy::expect_used)]

use context_engine::cache::*;
use context_engine::hash::*;
use context_engine::model::*;
use review_core::reviewer_type::ReviewerType;
use serde_json::json;

fn package(version: &str) -> ContextPackage {
    ContextPackage::new(
        ReviewerType::Security,
        "c1",
        vec![],
        vec![],
        ContextBudget {
            max_tokens: 100,
            max_items: 10,
        },
        vec![RiskSignal {
            kind: "authz".into(),
            detail: "d".into(),
            score: 0.5,
        }],
        ChangeSummary::default(),
        ContextProvenance {
            weights_version: version.into(),
            ..ContextProvenance::default()
        },
    )
}

fn parts<'a>() -> InputHashParts<'a> {
    InputHashParts {
        change_model_hash: "cm",
        impact_hash: "im",
        risk_hash: "rk",
        cluster_id: "c1",
        reviewer: ReviewerType::Security,
        budget: ContextBudget {
            max_tokens: 100,
            max_items: 10,
        },
        weights_version: "v1",
        profile_version: "p1",
        config_hash: "cfg",
        semantic_space: None,
    }
}

#[test]
fn canonical_json_sorts_keys_and_formats_floats() {
    let v = json!({"b": 1, "a": {"y": 0.5, "x": [true, null]}});
    assert_eq!(
        canonical_json(&v),
        r#"{"a":{"x":[true,null],"y":0.500000},"b":1}"#
    );
}

#[test]
fn package_hash_stable_across_runs() {
    assert_eq!(package_hash(&package("v1")), package_hash(&package("v1")));
    let sealed = seal(package("v1"));
    assert_eq!(sealed.package_hash, package_hash(&sealed));
}

#[test]
fn package_hash_changes_with_weights_version() {
    assert_ne!(package_hash(&package("v1")), package_hash(&package("v2")));
}

#[test]
fn input_hash_covers_all_inputs() {
    let base = input_hash(&parts());
    let variants = [
        InputHashParts {
            change_model_hash: "x",
            ..parts()
        },
        InputHashParts {
            impact_hash: "x",
            ..parts()
        },
        InputHashParts {
            risk_hash: "x",
            ..parts()
        },
        InputHashParts {
            cluster_id: "x",
            ..parts()
        },
        InputHashParts {
            reviewer: ReviewerType::Test,
            ..parts()
        },
        InputHashParts {
            budget: ContextBudget {
                max_tokens: 101,
                max_items: 10,
            },
            ..parts()
        },
        InputHashParts {
            budget: ContextBudget {
                max_tokens: 100,
                max_items: 11,
            },
            ..parts()
        },
        InputHashParts {
            weights_version: "x",
            ..parts()
        },
        InputHashParts {
            profile_version: "x",
            ..parts()
        },
        InputHashParts {
            config_hash: "x",
            ..parts()
        },
        InputHashParts {
            semantic_space: Some("hash-v1"),
            ..parts()
        },
    ];
    for v in variants {
        assert_ne!(input_hash(&v), base, "{v:?}");
    }
}

#[test]
fn cache_roundtrip_first_write_wins() {
    let cache = InMemoryContextCache::new();
    let key = CtxKey {
        review_run_id: "r1".into(),
        cluster_id: "c1".into(),
        reviewer: ReviewerType::Security,
        input_hash: "h".into(),
    };
    assert!(cache.get(&key).is_none());
    cache.put(&key, &package("v1"));
    cache.put(&key, &package("v2"));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(&key).unwrap().provenance.weights_version, "v1");
}
