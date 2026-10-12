//! REV-S-002: security reviewer routing and budget.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeSet;

use model_gateway::RiskBand;
use reviewers::reviewer::RoutingInput;
use reviewers::routing::{ChangeCluster, ClusterSymbol, SkipReason};
use reviewers::security::{security_applies, security_budget, security_reviewer};
use reviewers::{Applicability, Reviewer, ReviewerKind, RiskAssessment};

fn cluster(signals: &[&str], generated: bool) -> ChangeCluster {
    ChangeCluster {
        cluster_id: "c1".into(),
        risk_score: 0.5,
        signals: signals
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        symbols: vec![ClusterSymbol {
            symbol_id: "ts:src/a#A.f/method".into(),
            path: "src/a.ts".into(),
            generated,
        }],
        public_endpoint: false,
    }
}

#[test]
fn security_applies_on_auth_path_change() {
    let c = cluster(&["auth_path"], false);
    let a = security_applies(&RoutingInput {
        cluster: &c,
        risk: None,
    });
    assert!(matches!(a, Applicability::Applies { .. }));
}

#[test]
fn security_skips_docs_only_change() {
    let c = cluster(&["docs_only"], false);
    let a = security_applies(&RoutingInput {
        cluster: &c,
        risk: None,
    });
    assert_eq!(
        a,
        Applicability::Skip {
            reason: SkipReason::NotApplicable
        }
    );
}

#[test]
fn security_skips_generated_only_change() {
    let c = cluster(&["auth_path"], true);
    let a = security_applies(&RoutingInput {
        cluster: &c,
        risk: None,
    });
    assert_eq!(
        a,
        Applicability::Skip {
            reason: SkipReason::GeneratedOnly
        }
    );
}

#[test]
fn security_budget_scales_for_critical() {
    let mut risk = RiskAssessment::default();
    assert_eq!(security_budget(&risk).max_input_tokens, 12_000);
    risk.level = RiskBand::Critical;
    assert_eq!(security_budget(&risk).max_input_tokens, 18_000);
}

#[test]
fn security_reviewer_binds_prompt_v1() {
    let r = security_reviewer().expect("reviewer");
    assert_eq!(r.kind(), ReviewerKind::Security);
    assert_eq!(r.prompt().lock_key(), "security/v1");
}
