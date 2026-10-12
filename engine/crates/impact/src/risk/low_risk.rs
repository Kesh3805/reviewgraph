//! Low-risk change detection with contract-change override (RISK-006, PRD §39).
//!
//! Conservative by construction: any uncertainty (degraded parse, any change class, a public
//! rename with outside callers, a constant read by security code, a high/critical path rule)
//! resolves to "not low-risk" and the reason is recorded.

use std::collections::{BTreeMap, BTreeSet};

use review_core::change::SymbolChange;
use review_core::ids::SymbolKey;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::level::RiskLevel;
use super::policy::RiskPolicy;
use super::signal::{RiskCategory, RiskSignal, SignalSubject};
use crate::graph::{ImpactGraph, Relation};
use crate::input::{facts, ChangeSet, FileDisposition, FileInput, SymbolInput};

/// The kind of low-risk change.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LowRiskKind {
    FormattingOrComments,
    PureRename,
    Generated,
    ImportSort,
    SimpleConstant,
    SnapshotUpdate,
}

/// Why a nominally low-risk change is not low-risk.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OverrideReason {
    /// A public symbol renamed with callers outside the PR, or an API contract change.
    PublicContract,
    /// The symbol carries a change class.
    ChangeClass,
    /// A constant read by authorization/validation/crypto/payment code or a config default.
    SensitiveConstantReader,
    /// The file matches a `risk.paths` rule at high or critical.
    RiskPath,
    /// Side-effect import order changed.
    SideEffectImportOrder,
}

/// The verdict for one symbol or file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LowRiskVerdict {
    pub kind: Option<LowRiskKind>,
    pub overridden_by: Option<OverrideReason>,
}

impl LowRiskVerdict {
    /// Low-risk and not overridden.
    pub fn is_low_risk(&self) -> bool {
        self.kind.is_some() && self.overridden_by.is_none()
    }
}

/// Graph and policy facts the detector cannot read from the change set.
#[derive(Debug, Clone, Default)]
pub struct LowRiskContext {
    /// Changed symbols that have callers outside the PR (impact `Caller`s not in the change).
    pub symbols_with_external_callers: BTreeSet<SymbolKey>,
    /// Constants read by a symbol with Authorization/Validation/Cryptography/Payments signals
    /// or used as a config/env default.
    pub sensitive_constants: BTreeSet<SymbolKey>,
    /// Files matching a `risk.paths` rule at high or critical.
    pub high_risk_path_files: BTreeSet<RepoPath>,
}

impl LowRiskContext {
    /// Derives the context from the impact graph, the policy and the detected signals:
    /// changed symbols with callers outside the PR, constants reached by authorization,
    /// validation, cryptography or payment code (or by a config or environment node), and
    /// files under a `high` or `critical` configured risk path.
    pub fn derive(
        changes: &ChangeSet,
        impact: Option<&ImpactGraph>,
        policy: &RiskPolicy,
        signals: &[RiskSignal],
    ) -> Self {
        let mut context = Self::default();
        for file in &changes.files {
            let level = policy
                .paths
                .iter()
                .find(|rule| rule.matches(&file.path))
                .map(|rule| rule.level);
            if level.is_some_and(|level| level >= RiskLevel::High) {
                context.high_risk_path_files.insert(file.path.clone());
            }
        }
        let Some(impact) = impact else {
            return context;
        };
        let changed_ids: BTreeSet<&str> = changes.symbols.iter().map(SymbolInput::id).collect();
        let sensitive_symbols: BTreeSet<SymbolKey> = signals
            .iter()
            .filter(|signal| {
                matches!(
                    signal.category,
                    RiskCategory::Authorization
                        | RiskCategory::Validation
                        | RiskCategory::Cryptography
                        | RiskCategory::Payments
                )
            })
            .filter_map(|signal| match &signal.subject {
                SignalSubject::Symbol(key) => Some(*key),
                _ => None,
            })
            .collect();
        let sensitive_ids: BTreeSet<&str> = changes
            .symbols
            .iter()
            .filter(|symbol| sensitive_symbols.contains(&symbol.key()))
            .map(SymbolInput::id)
            .collect();
        for symbol_impact in &impact.symbols {
            let external_caller = symbol_impact
                .of(Relation::Caller)
                .any(|element| !changed_ids.contains(element.node_id.as_str()));
            if external_caller {
                context
                    .symbols_with_external_callers
                    .insert(symbol_impact.seed);
            }
            let sensitive_reader = symbol_impact.elements.iter().any(|element| {
                sensitive_ids.contains(element.node_id.as_str())
                    || matches!(element.relation, Relation::Config | Relation::EnvVar)
            });
            if sensitive_reader {
                context.sensitive_constants.insert(symbol_impact.seed);
            }
        }
        context
    }
}

/// Verdicts per symbol and file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LowRiskReport {
    pub per_symbol: BTreeMap<SymbolKey, LowRiskVerdict>,
    pub per_file: BTreeMap<RepoPath, LowRiskVerdict>,
}

impl LowRiskReport {
    pub fn symbol_is_low_risk(&self, key: &SymbolKey) -> bool {
        self.per_symbol
            .get(key)
            .is_some_and(LowRiskVerdict::is_low_risk)
    }

    pub fn file_is_low_risk(&self, path: &RepoPath) -> bool {
        self.per_file
            .get(path)
            .is_some_and(LowRiskVerdict::is_low_risk)
    }
}

fn symbol_kind(symbol: &SymbolInput) -> Option<LowRiskKind> {
    if symbol.generated {
        return Some(LowRiskKind::Generated);
    }
    match &symbol.symbol.change {
        SymbolChange::Modified {
            signature: false,
            body: false,
            attrs: false,
        } => Some(LowRiskKind::FormattingOrComments),
        SymbolChange::Renamed { similarity, .. }
            if *similarity >= 0.95 && !symbol.rename_body_changed =>
        {
            Some(LowRiskKind::PureRename)
        }
        _ if symbol.cosmetic => Some(LowRiskKind::FormattingOrComments),
        _ if symbol.facts.contains(facts::LITERAL_INITIALIZER_ONLY) => {
            Some(LowRiskKind::SimpleConstant)
        }
        _ => None,
    }
}

fn symbol_verdict(symbol: &SymbolInput, context: &LowRiskContext) -> LowRiskVerdict {
    if symbol.parse_degraded {
        return LowRiskVerdict::default();
    }
    let Some(kind) = symbol_kind(symbol) else {
        return LowRiskVerdict::default();
    };
    let key = symbol.key();
    let overridden_by = if symbol.has_class(crate::input::ChangeClass::ApiContractChanged) {
        Some(OverrideReason::PublicContract)
    } else if kind != LowRiskKind::Generated && !symbol.classes.is_empty() {
        Some(OverrideReason::ChangeClass)
    } else if kind == LowRiskKind::PureRename
        && symbol.exported
        && context.symbols_with_external_callers.contains(&key)
    {
        Some(OverrideReason::PublicContract)
    } else if kind == LowRiskKind::SimpleConstant && context.sensitive_constants.contains(&key) {
        Some(OverrideReason::SensitiveConstantReader)
    } else if kind != LowRiskKind::Generated && context.high_risk_path_files.contains(symbol.path())
    {
        Some(OverrideReason::RiskPath)
    } else {
        None
    };
    LowRiskVerdict {
        kind: Some(kind),
        overridden_by,
    }
}

fn is_snapshot(path: &RepoPath) -> bool {
    path.as_str().ends_with(".snap")
}

fn file_verdict(file: &FileInput, context: &LowRiskContext) -> LowRiskVerdict {
    if file.parse_degraded {
        return LowRiskVerdict::default();
    }
    let kind = if matches!(
        file.disposition,
        FileDisposition::Generated | FileDisposition::Vendored
    ) {
        Some(LowRiskKind::Generated)
    } else if is_snapshot(&file.path) {
        Some(LowRiskKind::SnapshotUpdate)
    } else if file.import_only.is_some_and(|i| i.bindings_equal) {
        Some(LowRiskKind::ImportSort)
    } else if file.cosmetic_only {
        Some(LowRiskKind::FormattingOrComments)
    } else if file.rename_similarity.is_some_and(|s| s >= 1.0) {
        Some(LowRiskKind::PureRename)
    } else {
        None
    };
    let Some(kind) = kind else {
        return LowRiskVerdict::default();
    };
    let overridden_by = if kind == LowRiskKind::ImportSort
        && file
            .import_only
            .is_some_and(|i| i.side_effect_order_changed)
    {
        Some(OverrideReason::SideEffectImportOrder)
    } else if kind != LowRiskKind::Generated && context.high_risk_path_files.contains(&file.path) {
        Some(OverrideReason::RiskPath)
    } else {
        None
    };
    LowRiskVerdict {
        kind: Some(kind),
        overridden_by,
    }
}

/// Classifies every changed symbol and file.
pub fn detect_low_risk(changes: &ChangeSet, context: &LowRiskContext) -> LowRiskReport {
    LowRiskReport {
        per_symbol: changes
            .symbols
            .iter()
            .map(|s| (s.key(), symbol_verdict(s, context)))
            .collect(),
        per_file: changes
            .files
            .iter()
            .map(|f| (f.path.clone(), file_verdict(f, context)))
            .collect(),
    }
}
