//! Pure router from `routing.yaml` (GW-006): `(tier x risk band x privacy)` to an ordered
//! candidate list.
//!
//! * `route()` is a pure function of `(table, registered providers, query)`.
//! * Override documents (organisation, then repository) replace whole
//!   `(tier, risk band, privacy)` rows; they never merge inside candidate lists, may not declare
//!   providers, and can only tighten `privacy_floor` (the most restrictive layer wins).
//! * An unset `${ENV}` model placeholder disables that candidate.
//! * The privacy filter fails closed: `no_external` with no self-hosted candidate yields
//!   `NoEligibleProvider`, never an external provider.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::builder::{RouteQuery, RouteSource};
use crate::error::{Error, GatewayError};
use crate::ratelimit::ModelLimits;
use crate::types::{ModelTier, PrivacyClass, ProviderId, RiskBand, RouteCandidate, RouteDecision};

/// The default table (ADR-010 provisional defaults).
pub const DEFAULT_ROUTING_YAML: &str = include_str!("../config/routing.default.yaml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderEntry {
    pub kind: ProviderKind,
    /// Runs inside the operator's own boundary; the only kind `no_external` may use.
    #[serde(default)]
    pub self_hosted: bool,
    /// Operator attestation of a zero-retention agreement. Defaults to false.
    #[serde(default)]
    pub zero_retention: bool,
    /// Account rate limits per model id (GW-007). A model without an entry is not limited.
    #[serde(default)]
    pub limits: BTreeMap<String, ModelLimits>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CandidateSpec {
    pub provider: String,
    /// Model id, or a `${ENV_VAR}` placeholder resolved at load time.
    pub model: String,
    pub max_context: u32,
    #[serde(default)]
    pub enabled_if_set: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RouteRow {
    pub tier: ModelTier,
    pub risk_bands: Vec<RiskBand>,
    pub privacy: Vec<PrivacyClass>,
    pub candidates: Vec<CandidateSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeepReasonerPolicy {
    pub min_risk_band: RiskBand,
    pub min_remaining_budget_fraction: f32,
    pub downgrade_to: ModelTier,
}

/// The `routing.yaml` document. Overrides use the same schema (providers must stay empty).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RoutingFile {
    pub version: u32,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderEntry>,
    #[serde(default)]
    pub routes: Vec<RouteRow>,
    #[serde(default)]
    pub deep_reasoner: Option<DeepReasonerPolicy>,
    /// Minimum privacy class applied to every request routed with this table.
    #[serde(default)]
    pub privacy_floor: Option<PrivacyClass>,
}

type RowKey = (ModelTier, RiskBand, PrivacyClass);

/// A routing table with env placeholders resolved and rows expanded.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingTable {
    providers: BTreeMap<String, ProviderEntry>,
    rows: BTreeMap<RowKey, Vec<RouteCandidate>>,
    deep: DeepReasonerPolicy,
    privacy_floor: PrivacyClass,
    table_hash: String,
}

fn config_err(msg: impl Into<String>) -> Error {
    Error::Config(msg.into())
}

/// Resolves a `${VAR}` placeholder; `None` means unset (the candidate is disabled).
fn resolve_model(model: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(var) = model.strip_prefix("${").and_then(|m| m.strip_suffix('}')) {
        lookup(var)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    } else {
        Some(model.to_owned())
    }
}

fn expand_rows(
    routes: &[RouteRow],
    providers: &dyn Fn(&str) -> bool,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<BTreeMap<RowKey, Vec<RouteCandidate>>, Error> {
    let mut rows: BTreeMap<RowKey, Vec<RouteCandidate>> = BTreeMap::new();
    for row in routes {
        let mut candidates = Vec::new();
        for c in &row.candidates {
            if !providers(&c.provider) {
                return Err(config_err(format!(
                    "route {:?} references unknown provider `{}`",
                    row.tier, c.provider
                )));
            }
            if let Some(model) = resolve_model(&c.model, lookup) {
                candidates.push(RouteCandidate {
                    provider: ProviderId::new(c.provider.clone()),
                    model,
                    max_context: c.max_context,
                    supports_reasoning: c.supports_reasoning,
                });
            }
        }
        for band in &row.risk_bands {
            for privacy in &row.privacy {
                let key = (row.tier, *band, *privacy);
                if rows.insert(key, candidates.clone()).is_some() {
                    return Err(config_err(format!(
                        "duplicate route row for tier {:?}, risk band {band:?}, privacy {privacy:?}",
                        row.tier
                    )));
                }
            }
        }
    }
    Ok(rows)
}

impl RoutingTable {
    /// Builds the table from a parsed document. `lookup` resolves `${ENV}` placeholders.
    pub fn from_file(
        file: &RoutingFile,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, Error> {
        if file.version != 1 {
            return Err(config_err(format!(
                "unsupported routing version {}",
                file.version
            )));
        }
        let deep = file
            .deep_reasoner
            .ok_or_else(|| config_err("deep_reasoner policy is required"))?;
        if deep.downgrade_to == ModelTier::DeepReasoner {
            return Err(config_err(
                "deep_reasoner.downgrade_to must be another tier",
            ));
        }
        let rows = expand_rows(&file.routes, &|p| file.providers.contains_key(p), lookup)?;
        Self::finish(
            file.providers.clone(),
            rows,
            deep,
            file.privacy_floor.unwrap_or(PrivacyClass::Standard),
        )
    }

    pub fn from_yaml(yaml: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let file: RoutingFile = serde_yaml::from_str(yaml)
            .map_err(|e| config_err(format!("invalid routing yaml: {e}")))?;
        Self::from_file(&file, lookup)
    }

    /// The built-in default table.
    pub fn default_table(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, Error> {
        Self::from_yaml(DEFAULT_ROUTING_YAML, lookup)
    }

    fn finish(
        providers: BTreeMap<String, ProviderEntry>,
        rows: BTreeMap<RowKey, Vec<RouteCandidate>>,
        deep: DeepReasonerPolicy,
        privacy_floor: PrivacyClass,
    ) -> Result<Self, Error> {
        let canonical = serde_json::json!({
            "providers": providers.iter().map(|(k, v)| serde_json::json!({
                "name": k, "kind": v.kind, "self_hosted": v.self_hosted, "zero_retention": v.zero_retention, "limits": v.limits,
            })).collect::<Vec<_>>(),
            "rows": rows.iter().map(|((t, b, p), c)| serde_json::json!({
                "tier": t, "risk_band": b, "privacy": p, "candidates": c,
            })).collect::<Vec<_>>(),
            "deep_reasoner": deep,
            "privacy_floor": privacy_floor,
        });
        Ok(Self {
            providers,
            rows,
            deep,
            privacy_floor,
            table_hash: crate::request_hash::hash_value(&canonical),
        })
    }

    pub fn table_hash(&self) -> &str {
        &self.table_hash
    }

    pub fn privacy_floor(&self) -> PrivacyClass {
        self.privacy_floor
    }

    pub fn provider(&self, name: &str) -> Option<&ProviderEntry> {
        self.providers.get(name)
    }

    /// Whether `provider` may receive a request of `privacy` class.
    pub fn provider_permits(&self, provider: &str, privacy: PrivacyClass) -> bool {
        match (self.providers.get(provider), privacy) {
            (Some(_), PrivacyClass::Standard) => true,
            (Some(p), PrivacyClass::ZeroRetentionOnly) => p.zero_retention || p.self_hosted,
            (Some(p), PrivacyClass::NoExternal) => p.self_hosted,
            (None, _) => false,
        }
    }

    /// Applies one override document. Rows replace by key; privacy can only tighten.
    pub fn with_override(
        &self,
        file: &RoutingFile,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, Error> {
        if file.version != 1 {
            return Err(config_err(format!(
                "unsupported routing version {}",
                file.version
            )));
        }
        if !file.providers.is_empty() {
            return Err(config_err("override documents may not declare providers"));
        }
        let replaced = expand_rows(&file.routes, &|p| self.providers.contains_key(p), lookup)?;
        let mut rows = self.rows.clone();
        rows.extend(replaced);
        let deep = file.deep_reasoner.unwrap_or(self.deep);
        if deep.downgrade_to == ModelTier::DeepReasoner {
            return Err(config_err(
                "deep_reasoner.downgrade_to must be another tier",
            ));
        }
        let floor = self
            .privacy_floor
            .max(file.privacy_floor.unwrap_or(PrivacyClass::Standard));
        Self::finish(self.providers.clone(), rows, deep, floor)
    }
}

/// Outcome of layering overrides on a base table.
#[derive(Debug)]
pub struct MergeOutcome {
    pub table: RoutingTable,
    /// Layers that were ignored, with the reason (the caller counts
    /// `routing_override_invalid_total` and logs them).
    pub rejected: Vec<(&'static str, String)>,
}

/// Layers `org` then `repo` override YAML on `base`. An invalid override is ignored (defaults
/// stay in force) and reported in `rejected`; it never loosens privacy.
pub fn merge_overrides(
    base: &RoutingTable,
    org: Option<&str>,
    repo: Option<&str>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> MergeOutcome {
    let mut table = base.clone();
    let mut rejected = Vec::new();
    for (layer, doc) in [("organization", org), ("repository", repo)] {
        let Some(doc) = doc else { continue };
        let applied = serde_yaml::from_str::<RoutingFile>(doc)
            .map_err(|e| config_err(format!("invalid override yaml: {e}")))
            .and_then(|f| table.with_override(&f, lookup));
        match applied {
            Ok(t) => table = t,
            Err(e) => rejected.push((layer, e.to_string())),
        }
    }
    MergeOutcome { table, rejected }
}

/// Pure routing decision. See the module docs for the resolution order.
pub fn route(
    table: &RoutingTable,
    registered: &HashSet<ProviderId>,
    q: &RouteQuery,
) -> Result<RouteDecision, GatewayError> {
    let privacy = q.privacy.max(table.privacy_floor);

    let mut effective = q.tier;
    let mut downgraded = None;
    if q.tier == ModelTier::DeepReasoner {
        let d = table.deep;
        if q.risk_band < d.min_risk_band {
            effective = d.downgrade_to;
            downgraded = Some(format!(
                "risk band {:?} is below {:?}",
                q.risk_band, d.min_risk_band
            ));
        } else if q.remaining_budget_fraction < d.min_remaining_budget_fraction {
            effective = d.downgrade_to;
            downgraded = Some(format!(
                "remaining budget {:.2} is below {:.2}",
                q.remaining_budget_fraction, d.min_remaining_budget_fraction
            ));
        }
    }

    let no_eligible = |reason: String| GatewayError::NoEligibleProvider {
        tier: effective,
        privacy,
        reason,
    };
    let row = table
        .rows
        .get(&(effective, q.risk_band, privacy))
        .ok_or_else(|| {
            no_eligible(format!(
                "no route for {effective:?} at risk {:?} with privacy {privacy:?}",
                q.risk_band
            ))
        })?;

    let needed = u64::from(q.est_input_tokens) + u64::from(q.max_output_tokens);
    let mut dropped = [0u32; 4];
    let candidates: Vec<RouteCandidate> = row
        .iter()
        .filter(|c| {
            if !registered.contains(&c.provider) {
                dropped[0] += 1;
                return false;
            }
            if needed > u64::from(c.max_context) {
                dropped[1] += 1;
                return false;
            }
            if !table.provider_permits(c.provider.as_str(), privacy) {
                dropped[2] += 1;
                return false;
            }
            if !q.schema_strict_ok.contains(&c.provider) {
                dropped[3] += 1;
                return false;
            }
            true
        })
        .cloned()
        .collect();

    if candidates.is_empty() {
        return Err(no_eligible(format!(
            "all candidates filtered (unregistered: {}, context: {}, privacy: {}, schema: {})",
            dropped[0], dropped[1], dropped[2], dropped[3]
        )));
    }
    Ok(RouteDecision {
        requested_tier: q.tier,
        effective_tier: effective,
        candidates,
        downgraded,
        table_hash: table.table_hash.clone(),
        attempted: Vec::new(),
    })
}

/// A [`RouteSource`] backed by a hot-swappable [`RoutingTable`]. In-flight calls keep the table
/// they started with.
#[derive(Debug)]
pub struct TableRouter {
    table: ArcSwap<RoutingTable>,
}

impl TableRouter {
    pub fn new(table: RoutingTable) -> Self {
        Self {
            table: ArcSwap::from_pointee(table),
        }
    }

    pub fn swap(&self, table: RoutingTable) {
        self.table.store(Arc::new(table));
    }

    pub fn current(&self) -> Arc<RoutingTable> {
        self.table.load_full()
    }
}

impl RouteSource for TableRouter {
    fn route(
        &self,
        registered: &HashSet<ProviderId>,
        query: &RouteQuery,
    ) -> Result<RouteDecision, GatewayError> {
        route(&self.table.load(), registered, query)
    }

    fn permits(&self, provider: &ProviderId, privacy: PrivacyClass) -> bool {
        let table = self.table.load();
        table.provider_permits(provider.as_str(), privacy.max(table.privacy_floor))
    }

    fn limits(&self, provider: &ProviderId, model: &str) -> Option<ModelLimits> {
        self.table
            .load()
            .provider(provider.as_str())
            .and_then(|p| p.limits.get(model).copied())
    }
}
