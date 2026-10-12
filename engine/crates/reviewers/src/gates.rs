//! Reviewer-specific output gates (REV-T-001, REV-A-001, REV-P-001, REV-M-001).
//!
//! The model is asked for evidence; these gates check it against the context the model was shown
//! and drop a candidate that does not hold up: silence over speculation. A gate never sets a
//! confidence (ADR-011) and never rewrites a candidate. Dropped candidates are returned so the
//! pipeline can persist them as `REJECTED_INVALID` with the gate's reason.
//!
//! | Reviewer | Gate |
//! |---|---|
//! | tests | cites a mapped test (`T#`) or states `no_mapped_test` with `D#` mapping evidence |
//! | architecture | rests on a basis that exists in the rules; a convention needs confidence >= 0.9 and >= 10 samples (as the rule text states them) |
//! | performance | names an `execution_path` whose every hop is a `CALLS` or `HANDLED_BY` edge of confidence >= 0.6 |
//! | maintainability | cites a deterministic seed (`D#`) whose metric matches the category; severity never above `medium` |

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::architecture::{basis_usable, BasisKind, ARCHITECTURE_CATEGORIES};
use crate::context::ReviewContext;
use crate::maintainability::{MAINTAINABILITY_CATEGORIES, SEED_TOOL};
use crate::output::RawItem;
use crate::performance::{COST_DRIVERS, MIN_PATH_EDGE_CONFIDENCE};
use crate::refs::{RefKind, RefTable};
use crate::reviewer::ReviewerKind;
use crate::tests_reviewer::TESTS_CATEGORIES;

/// Why a candidate was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GateReason {
    /// The category is not one of the reviewer's categories.
    CategoryNotAllowed,
    /// The candidate cites neither a mapped test nor mapping evidence.
    MissingTestEvidence,
    /// The candidate states no basis.
    MissingBasis,
    /// The basis id or rule ref is not in the provided rules (`unknown_basis`).
    UnknownBasis,
    /// A convention basis is below the confidence or sample floor.
    ConventionBelowThreshold,
    /// The execution path is missing, or a hop is not a proven edge (`unproven_path`).
    UnprovenPath,
    /// The candidate cites no deterministic seed of the right kind.
    MissingSeed,
    /// The severity is not allowed for this reviewer.
    SeverityNotAllowed,
}

impl GateReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CategoryNotAllowed => "category_not_allowed",
            Self::MissingTestEvidence => "missing_test_evidence",
            Self::MissingBasis => "missing_basis",
            Self::UnknownBasis => "unknown_basis",
            Self::ConventionBelowThreshold => "convention_below_threshold",
            Self::UnprovenPath => "unproven_path",
            Self::MissingSeed => "missing_seed",
            Self::SeverityNotAllowed => "severity_not_allowed",
        }
    }
}

/// A candidate dropped by a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatedItem {
    /// Position in the model output.
    pub ordinal: usize,
    pub reason: GateReason,
}

static CONFIDENCE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)confidence\D{0,3}([0-9]*\.?[0-9]+)").ok());
static SAMPLES_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)samples\D{0,3}([0-9]+)|([0-9]+)\s+samples").ok());

/// Confidence and sample count a rule's text states for itself, when it does.
pub fn rule_stats(text: &str) -> (Option<f64>, Option<u32>) {
    let confidence = CONFIDENCE_RE
        .as_ref()
        .and_then(|re| re.captures(text))
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok());
    let samples = SAMPLES_RE
        .as_ref()
        .and_then(|re| re.captures(text))
        .and_then(|c| c.get(1).or_else(|| c.get(2)))
        .and_then(|m| m.as_str().parse::<u32>().ok());
    (confidence, samples)
}

fn str_field<'a>(json: &'a Value, key: &str) -> Option<&'a str> {
    json.get(key).and_then(Value::as_str)
}

fn category_allowed(json: &Value, allowed: &[&str]) -> bool {
    str_field(json, "category").is_some_and(|c| allowed.contains(&c))
}

fn tests_gate(json: &Value, refs: &RefTable) -> Option<GateReason> {
    if !category_allowed(json, TESTS_CATEGORIES) {
        return Some(GateReason::CategoryNotAllowed);
    }
    let kind_of = |r: &str| refs.get(r).map(|e| e.kind);
    if let Some(test_ref) = str_field(json, "test_case_ref") {
        return (kind_of(test_ref) != Some(RefKind::Test))
            .then_some(GateReason::MissingTestEvidence);
    }
    let no_mapped = json
        .get("no_mapped_test")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let evidence = str_field(json, "mapping_evidence_ref").and_then(kind_of);
    if no_mapped && evidence == Some(RefKind::Diagnostic) {
        None
    } else {
        Some(GateReason::MissingTestEvidence)
    }
}

fn architecture_gate(json: &Value, cx: &dyn ReviewContext, refs: &RefTable) -> Option<GateReason> {
    if !category_allowed(json, ARCHITECTURE_CATEGORIES) {
        return Some(GateReason::CategoryNotAllowed);
    }
    let Some(basis) = json.get("basis").filter(|b| b.is_object()) else {
        return Some(GateReason::MissingBasis);
    };
    let id = str_field(basis, "id").unwrap_or("");
    let rule_ref = str_field(basis, "rule_ref").unwrap_or("");
    let rule_entry = refs.get(rule_ref).filter(|e| e.kind == RefKind::Rule);
    let rule = cx.rules().iter().find(|r| r.rule_id == id);
    let (Some(entry), Some(rule)) = (rule_entry, rule) else {
        return Some(GateReason::UnknownBasis);
    };
    if entry.target != id {
        return Some(GateReason::UnknownBasis);
    }
    let kind = match str_field(basis, "kind") {
        Some("explicit_rule") => BasisKind::ExplicitRule,
        Some("documented") => BasisKind::Documented,
        Some("convention") => BasisKind::Convention,
        _ => return Some(GateReason::MissingBasis),
    };
    if kind == BasisKind::Convention {
        let stated_confidence = basis
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let stated_samples = basis
            .get("samples")
            .and_then(Value::as_u64)
            .map_or(0, |s| u32::try_from(s).unwrap_or(u32::MAX));
        // The rule text is authoritative when it states a value: the model cannot raise it.
        let (text_confidence, text_samples) = rule_stats(&rule.text);
        let confidence = text_confidence.unwrap_or(stated_confidence);
        let samples = text_samples.unwrap_or(stated_samples);
        if !basis_usable(kind, confidence, samples) {
            return Some(GateReason::ConventionBelowThreshold);
        }
    }
    None
}

fn path_edge_kind(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("CALLS") || kind.eq_ignore_ascii_case("HANDLED_BY")
}

fn performance_gate(json: &Value, cx: &dyn ReviewContext, refs: &RefTable) -> Option<GateReason> {
    if !category_allowed(json, &["performance"]) {
        return Some(GateReason::CategoryNotAllowed);
    }
    if !str_field(json, "cost_driver").is_some_and(|d| COST_DRIVERS.contains(&d)) {
        return Some(GateReason::UnprovenPath);
    }
    let Some(path) = json.get("execution_path").and_then(Value::as_array) else {
        return Some(GateReason::UnprovenPath);
    };
    let mut targets: Vec<&str> = Vec::with_capacity(path.len());
    for hop in path {
        match hop
            .as_str()
            .and_then(|r| refs.get(r))
            .filter(|e| e.kind.is_symbol())
        {
            Some(entry) => targets.push(entry.target.as_str()),
            None => return Some(GateReason::UnprovenPath),
        }
    }
    if targets.is_empty() {
        return Some(GateReason::UnprovenPath);
    }
    for pair in targets.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let proven = cx.edges().iter().any(|e| {
            e.confidence >= MIN_PATH_EDGE_CONFIDENCE
                && path_edge_kind(&e.kind)
                && ((e.from == from && e.to == to)
                    // HANDLED_BY points from the endpoint to its handler in either reading.
                    || (e.kind.eq_ignore_ascii_case("HANDLED_BY") && e.from == to && e.to == from))
        });
        if !proven {
            return Some(GateReason::UnprovenPath);
        }
    }
    None
}

fn maintainability_gate(
    json: &Value,
    cx: &dyn ReviewContext,
    refs: &RefTable,
) -> Option<GateReason> {
    let Some(category) =
        str_field(json, "category").filter(|c| MAINTAINABILITY_CATEGORIES.contains(c))
    else {
        return Some(GateReason::CategoryNotAllowed);
    };
    if !matches!(str_field(json, "severity"), Some("medium" | "low")) {
        return Some(GateReason::SeverityNotAllowed);
    }
    let seed_prefix = format!("{SEED_TOOL}:");
    let seed_code = str_field(json, "seed_ref")
        .and_then(|r| refs.get(r))
        .filter(|e| e.kind == RefKind::Diagnostic)
        .and_then(|e| e.target.strip_prefix(&seed_prefix));
    let Some(code) = seed_code else {
        return Some(GateReason::MissingSeed);
    };
    let matches_category = match category {
        "duplicated_logic" => code == "duplicate",
        "unreviewable_function" => code == "cyclomatic_complexity" || code == "function_length",
        "unreadable_control_flow" => code == "nesting_depth",
        _ => true,
    };
    if !matches_category {
        return Some(GateReason::MissingSeed);
    }
    if category == "inconsistent_pattern" && !cites_usable_convention(json, cx, refs) {
        return Some(GateReason::ConventionBelowThreshold);
    }
    None
}

/// True when the candidate cites a rule whose text states a convention confidence and sample
/// count above the floors.
fn cites_usable_convention(json: &Value, cx: &dyn ReviewContext, refs: &RefTable) -> bool {
    json.get("affected_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|r| refs.get(r))
        .filter(|e| e.kind == RefKind::Rule)
        .filter_map(|e| cx.rules().iter().find(|rule| rule.rule_id == e.target))
        .any(|rule| {
            let (confidence, samples) = rule_stats(&rule.text);
            basis_usable(
                BasisKind::Convention,
                confidence.unwrap_or(0.0),
                samples.unwrap_or(0),
            )
        })
}

/// The gate verdict for one raw candidate; `None` keeps it.
pub fn gate(
    kind: ReviewerKind,
    json: &Value,
    cx: &dyn ReviewContext,
    refs: &RefTable,
) -> Option<GateReason> {
    match kind {
        ReviewerKind::Test => tests_gate(json, refs),
        ReviewerKind::Architecture => architecture_gate(json, cx, refs),
        ReviewerKind::Performance => performance_gate(json, cx, refs),
        ReviewerKind::Maintainability => maintainability_gate(json, cx, refs),
        ReviewerKind::Correctness | ReviewerKind::Security => None,
    }
}

/// Splits `items` into the candidates that pass the reviewer's gates and those dropped.
pub fn apply_gates(
    kind: ReviewerKind,
    items: Vec<RawItem>,
    cx: &dyn ReviewContext,
    refs: &RefTable,
) -> (Vec<RawItem>, Vec<GatedItem>) {
    let mut kept = Vec::with_capacity(items.len());
    let mut gated = Vec::new();
    for item in items {
        match gate(kind, &item.json, cx, refs) {
            None => kept.push(item),
            Some(reason) => gated.push(GatedItem {
                ordinal: item.ordinal,
                reason,
            }),
        }
    }
    (kept, gated)
}
