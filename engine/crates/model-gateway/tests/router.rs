#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use model_gateway::router::DEFAULT_ROUTING_YAML;
use model_gateway::{
    merge_overrides, route, FinishReason, GatewayBuilder, GatewayError, ModelGateway, ModelOutput,
    ModelTier, PermanentKind, PrivacyClass, ProviderAdapter, ProviderId, ProviderRequest,
    ProviderResponse, RiskBand, RouteQuery, RoutingFile, RoutingTable, ServedFrom, TableRouter,
    Usage,
};
use proptest::prelude::*;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use common::request;

fn env_with(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k: &str| {
        vars.iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| (*v).to_owned())
    }
}

fn none(_: &str) -> Option<String> {
    None
}

fn providers(names: &[&str]) -> HashSet<ProviderId> {
    names.iter().map(|n| ProviderId::new(*n)).collect()
}

fn query(tier: ModelTier, band: RiskBand, privacy: PrivacyClass) -> RouteQuery {
    RouteQuery {
        tier,
        risk_band: band,
        privacy,
        est_input_tokens: 10_000,
        max_output_tokens: 4_000,
        remaining_budget_fraction: 1.0,
        schema_strict_ok: providers(&["anthropic", "openai"]),
    }
}

fn default_table() -> RoutingTable {
    RoutingTable::default_table(&none).expect("default table")
}

fn models(d: &model_gateway::RouteDecision) -> Vec<String> {
    d.candidates.iter().map(|c| c.model.clone()).collect()
}

#[test]
fn routes_review_reasoner_to_sonnet_by_default() {
    let t = default_table();
    let reg = providers(&["anthropic", "openai"]);
    let expect = [
        (ModelTier::Classifier, "claude-haiku-4-5"),
        (ModelTier::FastReasoner, "claude-haiku-4-5"),
        (ModelTier::ReviewReasoner, "claude-sonnet-5-5"),
        (ModelTier::Verifier, "claude-sonnet-5-5"),
    ];
    for (tier, model) in expect {
        let d = route(
            &t,
            &reg,
            &query(tier, RiskBand::Medium, PrivacyClass::Standard),
        )
        .expect("route");
        assert_eq!(models(&d), vec![model.to_owned()], "{tier:?}");
        assert_eq!(d.effective_tier, tier);
        assert!(d.downgraded.is_none());
    }
    let d = route(
        &t,
        &reg,
        &query(
            ModelTier::DeepReasoner,
            RiskBand::Critical,
            PrivacyClass::Standard,
        ),
    )
    .expect("deep");
    assert_eq!(models(&d), vec!["claude-opus-5-5".to_owned()]);
    assert_eq!(d.table_hash, t.table_hash());
}

#[test]
fn only_anthropic_key_means_no_openai_candidates() {
    let t = RoutingTable::default_table(&env_with(&[("OPENAI_REVIEW_MODEL", "some-model")]))
        .expect("t");
    let q = query(
        ModelTier::ReviewReasoner,
        RiskBand::Low,
        PrivacyClass::Standard,
    );
    let only_anthropic = route(&t, &providers(&["anthropic"]), &q).expect("route");
    assert!(only_anthropic
        .candidates
        .iter()
        .all(|c| c.provider.as_str() == "anthropic"));
    let both = route(&t, &providers(&["anthropic", "openai"]), &q).expect("route");
    assert_eq!(
        models(&both),
        vec!["claude-sonnet-5-5".to_owned(), "some-model".to_owned()]
    );
}

#[test]
fn unset_env_placeholder_disables_candidate() {
    let q = query(
        ModelTier::ReviewReasoner,
        RiskBand::Low,
        PrivacyClass::Standard,
    );
    let d = route(&default_table(), &providers(&["anthropic", "openai"]), &q).expect("route");
    assert_eq!(d.candidates.len(), 1);
    assert_eq!(d.candidates[0].provider.as_str(), "anthropic");
}

#[test]
fn deep_reasoner_downgrades_below_critical() {
    let d = route(
        &default_table(),
        &providers(&["anthropic"]),
        &query(
            ModelTier::DeepReasoner,
            RiskBand::High,
            PrivacyClass::Standard,
        ),
    )
    .expect("route");
    assert_eq!(d.requested_tier, ModelTier::DeepReasoner);
    assert_eq!(d.effective_tier, ModelTier::ReviewReasoner);
    assert!(d.downgraded.is_some());
    assert_eq!(models(&d), vec!["claude-sonnet-5-5".to_owned()]);
}

#[test]
fn deep_reasoner_downgrades_on_low_budget() {
    let mut q = query(
        ModelTier::DeepReasoner,
        RiskBand::Critical,
        PrivacyClass::Standard,
    );
    q.remaining_budget_fraction = 0.3;
    let d = route(&default_table(), &providers(&["anthropic"]), &q).expect("route");
    assert_eq!(d.effective_tier, ModelTier::ReviewReasoner);
    assert!(d.downgraded.expect("reason").contains("budget"));
}

#[test]
fn no_external_fails_closed() {
    let err = route(
        &default_table(),
        &providers(&["anthropic", "openai"]),
        &query(
            ModelTier::ReviewReasoner,
            RiskBand::Low,
            PrivacyClass::NoExternal,
        ),
    )
    .expect_err("fail closed");
    assert!(matches!(
        err,
        GatewayError::NoEligibleProvider {
            privacy: PrivacyClass::NoExternal,
            ..
        }
    ));
}

#[test]
fn zero_retention_filters_providers() {
    let q = query(
        ModelTier::ReviewReasoner,
        RiskBand::Low,
        PrivacyClass::ZeroRetentionOnly,
    );
    let env = env_with(&[("OPENAI_REVIEW_MODEL", "m")]);
    let reg = providers(&["anthropic", "openai"]);
    // Nothing is attested: fail closed.
    let t = RoutingTable::default_table(&env).expect("t");
    assert!(route(&t, &reg, &q).is_err());
    // Attest openai only.
    let yaml = DEFAULT_ROUTING_YAML.replace(
        "openai: { kind: openai, self_hosted: false, zero_retention: false }",
        "openai: { kind: openai, self_hosted: false, zero_retention: true }",
    );
    let t = RoutingTable::from_yaml(&yaml, &env).expect("t");
    let d = route(&t, &reg, &q).expect("route");
    assert_eq!(models(&d), vec!["m".to_owned()]);
}

#[test]
fn context_window_filter_excludes_small_models() {
    let env = env_with(&[("OPENAI_REVIEW_MODEL", "small")]);
    let t = RoutingTable::default_table(&env).expect("t");
    let mut q = query(
        ModelTier::ReviewReasoner,
        RiskBand::Low,
        PrivacyClass::Standard,
    );
    q.est_input_tokens = 150_000; // fits 200k but not 128k
    let d = route(&t, &providers(&["anthropic", "openai"]), &q).expect("route");
    assert_eq!(models(&d), vec!["claude-sonnet-5-5".to_owned()]);
    q.est_input_tokens = 250_000;
    assert!(route(&t, &providers(&["anthropic", "openai"]), &q).is_err());
}

#[test]
fn strict_schema_filter_excludes_openai() {
    let env = env_with(&[("OPENAI_REVIEW_MODEL", "m")]);
    let t = RoutingTable::default_table(&env).expect("t");
    let mut q = query(
        ModelTier::ReviewReasoner,
        RiskBand::Low,
        PrivacyClass::Standard,
    );
    q.schema_strict_ok = providers(&["anthropic"]);
    let d = route(&t, &providers(&["anthropic", "openai"]), &q).expect("route");
    assert_eq!(d.candidates.len(), 1);
}

#[test]
fn duplicate_route_rows_rejected() {
    let yaml = format!(
        "{}\n  - tier: classifier\n    risk_bands: [low]\n    privacy: [standard]\n    candidates: []\n",
        DEFAULT_ROUTING_YAML.replace("deep_reasoner:\n  min_risk_band", "DEEP_MARK:\n  min_risk_band")
    );
    // Re-attach the policy after the extra row (rows must precede it in the YAML list).
    let yaml = yaml.replace("DEEP_MARK:", "deep_reasoner:");
    let err = RoutingTable::from_yaml(&yaml, &none);
    assert!(err.is_err(), "duplicate rows must be rejected");
}

#[test]
fn unknown_provider_and_bad_version_rejected() {
    let yaml = DEFAULT_ROUTING_YAML.replace("provider: openai,", "provider: nope,");
    assert!(RoutingTable::from_yaml(&yaml, &none).is_err());
    assert!(RoutingTable::from_yaml(
        &DEFAULT_ROUTING_YAML.replace("version: 1", "version: 2"),
        &none
    )
    .is_err());
}

#[test]
fn override_replaces_whole_rows() {
    let base = default_table();
    let org = r#"
version: 1
routes:
  - tier: review_reasoner
    risk_bands: [high]
    privacy: [standard]
    candidates:
      - { provider: anthropic, model: claude-opus-5-5, max_context: 200000 }
"#;
    let out = merge_overrides(&base, Some(org), None, &none);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    let reg = providers(&["anthropic"]);
    let high = route(
        &out.table,
        &reg,
        &query(
            ModelTier::ReviewReasoner,
            RiskBand::High,
            PrivacyClass::Standard,
        ),
    )
    .expect("route");
    assert_eq!(models(&high), vec!["claude-opus-5-5".to_owned()]);
    let low = route(
        &out.table,
        &reg,
        &query(
            ModelTier::ReviewReasoner,
            RiskBand::Low,
            PrivacyClass::Standard,
        ),
    )
    .expect("route");
    assert_eq!(models(&low), vec!["claude-sonnet-5-5".to_owned()]);
    assert_ne!(out.table.table_hash(), base.table_hash());
}

#[test]
fn repo_override_cannot_widen_privacy() {
    let base = default_table();
    let org = "version: 1\nprivacy_floor: no_external\n";
    let repo = "version: 1\nprivacy_floor: standard\n";
    let out = merge_overrides(&base, Some(org), Some(repo), &none);
    assert_eq!(out.table.privacy_floor(), PrivacyClass::NoExternal);
    // A plain request is raised to the floor and fails closed.
    let err = route(
        &out.table,
        &providers(&["anthropic"]),
        &query(ModelTier::Verifier, RiskBand::Low, PrivacyClass::Standard),
    )
    .expect_err("closed");
    assert!(matches!(
        err,
        GatewayError::NoEligibleProvider {
            privacy: PrivacyClass::NoExternal,
            ..
        }
    ));
}

#[test]
fn invalid_override_is_ignored_and_reported() {
    let base = default_table();
    let with_providers = "version: 1\nproviders:\n  evil: { kind: openai, self_hosted: true }\n";
    let out = merge_overrides(
        &base,
        Some(with_providers),
        Some("this: [is not valid"),
        &none,
    );
    assert_eq!(out.rejected.len(), 2);
    assert_eq!(out.table, base);
}

#[test]
fn hot_swap_keeps_in_flight_table() {
    let router = TableRouter::new(default_table());
    let before = router.current();
    router
        .swap(RoutingTable::default_table(&env_with(&[("OPENAI_REVIEW_MODEL", "x")])).expect("t"));
    assert_ne!(before.table_hash(), router.current().table_hash());
}

#[test]
fn routing_schema_matches_checked_in_file() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/routing.v1.schema.json");
    let s = schemars::gen::SchemaGenerator::default().into_root_schema_for::<RoutingFile>();
    let mut text =
        serde_json::to_string_pretty(&serde_json::to_value(s).expect("schema")).expect("render");
    text.push('\n');
    if std::env::var("UPDATE_SCHEMAS").is_ok() {
        std::fs::write(&path, &text).expect("write");
    }
    assert_eq!(
        std::fs::read_to_string(&path).expect("file; run with UPDATE_SCHEMAS=1"),
        text
    );
}

struct Scripted {
    provider: &'static str,
    fail: Option<fn() -> GatewayError>,
}

#[async_trait]
impl ProviderAdapter for Scripted {
    fn provider(&self) -> ProviderId {
        ProviderId::new(self.provider)
    }

    async fn send(&self, _req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        if let Some(f) = self.fail {
            return Err(f());
        }
        Ok(ProviderResponse {
            output: ModelOutput::Json(json!({"ok": true})),
            usage: Usage::default(),
            finish_reason: FinishReason::Complete,
            model: "m".into(),
            provider_request_id: None,
            tool_use_id: None,
            served_from: ServedFrom::Live,
        })
    }
}

fn two_provider_gateway(anthropic_fail: Option<fn() -> GatewayError>) -> model_gateway::Gateway {
    let table =
        RoutingTable::default_table(&env_with(&[("OPENAI_REVIEW_MODEL", "oa-model")])).expect("t");
    GatewayBuilder::new()
        .redactor(Arc::new(model_gateway::DefaultRedactor::new()))
        .adapter(Arc::new(Scripted {
            provider: "anthropic",
            fail: anthropic_fail,
        }))
        .adapter(Arc::new(Scripted {
            provider: "openai",
            fail: None,
        }))
        .router(Arc::new(TableRouter::new(table)))
        .build()
        .expect("build")
}

#[tokio::test]
async fn fallback_on_quota_to_next_candidate() {
    let gw = two_provider_gateway(Some(|| GatewayError::Permanent {
        kind: PermanentKind::QuotaExhausted,
        provider: Some(ProviderId::new("anthropic")),
        detail: "quota".into(),
    }));
    let resp = gw
        .call(request(), CancellationToken::new())
        .await
        .expect("fell back");
    assert_eq!(resp.provider.as_str(), "openai");
    assert_eq!(resp.route.attempted.len(), 1);
    assert_eq!(resp.route.attempted[0].provider.as_str(), "anthropic");
    assert_eq!(resp.route.attempted[0].error_class, "permanent");
}

#[tokio::test]
async fn no_fallback_on_invalid_request() {
    let gw = two_provider_gateway(Some(|| GatewayError::Permanent {
        kind: PermanentKind::InvalidRequest,
        provider: Some(ProviderId::new("anthropic")),
        detail: "bad".into(),
    }));
    let err = gw
        .call(request(), CancellationToken::new())
        .await
        .expect_err("no fallback");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::InvalidRequest,
            ..
        }
    ));
}

#[tokio::test]
async fn no_external_request_never_reaches_a_provider() {
    let gw = two_provider_gateway(None);
    let req = request().with_privacy(PrivacyClass::NoExternal);
    let err = gw
        .call(req, CancellationToken::new())
        .await
        .expect_err("closed");
    assert!(matches!(err, GatewayError::NoEligibleProvider { .. }));
}

proptest! {
    #[test]
    fn route_is_deterministic(
        tier in 0usize..5, band in 0usize..4, privacy in 0usize..3,
        tokens in 0u32..300_000, frac in 0.0f32..1.0, with_openai in any::<bool>(),
    ) {
        let tiers = [ModelTier::Classifier, ModelTier::FastReasoner, ModelTier::ReviewReasoner, ModelTier::Verifier, ModelTier::DeepReasoner];
        let bands = [RiskBand::Low, RiskBand::Medium, RiskBand::High, RiskBand::Critical];
        let privs = [PrivacyClass::Standard, PrivacyClass::ZeroRetentionOnly, PrivacyClass::NoExternal];
        let t = RoutingTable::default_table(&env_with(&[("OPENAI_REVIEW_MODEL", "x"), ("OPENAI_DEEP_MODEL", "y")])).unwrap();
        let reg = if with_openai { providers(&["anthropic", "openai"]) } else { providers(&["anthropic"]) };
        let mut q = query(tiers[tier], bands[band], privs[privacy]);
        q.est_input_tokens = tokens;
        q.remaining_budget_fraction = frac;
        let a = route(&t, &reg, &q);
        let b = route(&t, &reg, &q);
        prop_assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
}
