//! Candidate normalisation (REV-C-003): resolve refs to symbols and ranges, validate anchors and
//! evidence against the ref table, and reject unresolvable candidates visibly.
//!
//! Normalisation checks only *referential* validity, never truth (that is verification's job).
//! Paths always come from the ref table, never from model output. Every failure is a
//! [`Rejection`] value; nothing here panics.
//!
//! The output is a reviewer-owned [`NormalizedCandidate`] that carries what DOM-006's
//! `CandidateFinding` does not yet have (predicate, claimed relations, anchor side, normalised
//! claim, raw-output hash). VER-001 maps it onto the persisted candidate row.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use model_gateway::{OutputValidator, SchemaErrorSummary};
use review_core::evidence::EvidenceKind;
use review_core::finding::{FindingCategory, Severity};
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::DiffSide;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claim_text;
use crate::output::RawItem;
use crate::refs::{RefEntry, RefKind, RefTable};
use crate::reviewer::ReviewerKind;

/// Rejection taxonomy. A rejected candidate is persisted in state `REJECTED_INVALID` with its
/// code and raw JSON (VER-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RejectionCode {
    UnresolvableReference,
    LineOutOfRange,
    EvidenceMissing,
    AnchorNotSymbol,
}

impl RejectionCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnresolvableReference => "UNRESOLVABLE_REFERENCE",
            Self::LineOutOfRange => "LINE_OUT_OF_RANGE",
            Self::EvidenceMissing => "EVIDENCE_MISSING",
            Self::AnchorNotSymbol => "ANCHOR_NOT_SYMBOL",
        }
    }
}

impl fmt::Display for RejectionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A candidate that could not be normalised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rejection {
    pub code: RejectionCode,
    pub detail: String,
    pub ordinal: usize,
    pub raw_output_hash: String,
    pub raw_json: Value,
}

/// The resolved anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub symbol_id: SymbolId,
    pub symbol_key: SymbolKey,
    pub path: String,
    pub side: DiffSide,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedEvidence {
    pub kind: EvidenceKind,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub symbol_id: Option<SymbolId>,
    pub path: String,
    pub side: DiffSide,
    pub lines: Option<[u32; 2]>,
    pub quote: String,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedRelation {
    /// Node key: a `SymbolId` for symbol refs, otherwise the item id.
    pub from: String,
    pub relation: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Predicate {
    pub kind: String,
    pub subject: String,
    pub params: BTreeMap<String, String>,
}

/// A referentially valid candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedCandidate {
    pub ordinal: usize,
    pub reviewer: ReviewerKind,
    pub category: FindingCategory,
    /// The category exactly as the model wrote it (security categories map to `security`).
    pub model_category: String,
    pub severity: Severity,
    pub title: String,
    pub claim: String,
    pub description: String,
    pub claim_text_normalized: String,
    pub anchor: Anchor,
    /// Anchor symbol first, then the other cited symbols, without duplicates.
    pub affected_symbols: Vec<SymbolId>,
    /// Non-symbol refs (`T#`, `R#`, `D#`, `C#`) cited as affected: kept as evidence links.
    pub linked_items: Vec<String>,
    pub evidence: Vec<NormalizedEvidence>,
    pub claimed_relations: Vec<NormalizedRelation>,
    pub predicate: Predicate,
    pub corrective_direction: String,
    /// Informational only (ADR-011).
    pub self_confidence: f64,
    pub raw_output_hash: String,
    /// Normalisation notes (`anchor_clamped`, `evidence_dropped`, `relation_dropped`, ...).
    pub notes: Vec<String>,
}

/// Semantic validator run inside the gateway, so the model gets one repair turn for invented
/// refs (GW-009). Messages never echo model text.
#[derive(Debug, Clone)]
pub struct RefValidator {
    refs: Arc<RefTable>,
}

impl RefValidator {
    pub fn new(refs: Arc<RefTable>) -> Self {
        Self { refs }
    }
}

fn err(path: String, keyword: &str, message: &str) -> SchemaErrorSummary {
    SchemaErrorSummary {
        instance_path: path,
        keyword: keyword.to_owned(),
        message: message.to_owned(),
    }
}

impl OutputValidator for RefValidator {
    fn validate(&self, output: &Value) -> Vec<SchemaErrorSummary> {
        let mut errors = Vec::new();
        let Some(findings) = output.get("findings").and_then(Value::as_array) else {
            return errors;
        };
        let known = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .is_some_and(|r| self.refs.contains(r))
        };
        for (i, f) in findings.iter().enumerate() {
            let anchor = f.get("anchor");
            let anchor_ref = anchor.and_then(|a| a.get("ref"));
            if !known(anchor_ref) {
                errors.push(err(
                    format!("/findings/{i}/anchor/ref"),
                    "ref",
                    "unknown ref; cite a ref from the input",
                ));
            } else if anchor_ref
                .and_then(Value::as_str)
                .and_then(RefKind::from_ref)
                .is_some_and(|k| !k.is_symbol())
            {
                errors.push(err(
                    format!("/findings/{i}/anchor/ref"),
                    "ref",
                    "the anchor must be a changed symbol (S#) or a neighbour (N#)",
                ));
            }
            let start = anchor
                .and_then(|a| a.get("start_line"))
                .and_then(Value::as_u64);
            let end = anchor
                .and_then(|a| a.get("end_line"))
                .and_then(Value::as_u64);
            if let (Some(s), Some(e)) = (start, end) {
                if s > e {
                    errors.push(err(
                        format!("/findings/{i}/anchor"),
                        "range",
                        "start_line is after end_line",
                    ));
                }
            }
            for (j, r) in f
                .get("affected_refs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if !known(Some(r)) {
                    errors.push(err(
                        format!("/findings/{i}/affected_refs/{j}"),
                        "ref",
                        "unknown ref",
                    ));
                }
            }
            for (j, ev) in f
                .get("evidence")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if !known(ev.get("ref")) {
                    errors.push(err(
                        format!("/findings/{i}/evidence/{j}/ref"),
                        "ref",
                        "unknown ref",
                    ));
                }
            }
            for (j, rel) in f
                .get("claimed_relations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                for side in ["from", "to"] {
                    if !known(rel.get(side)) {
                        errors.push(err(
                            format!("/findings/{i}/claimed_relations/{j}/{side}"),
                            "ref",
                            "unknown ref",
                        ));
                    }
                }
            }
        }
        errors
    }
}

fn evidence_kind(kind: &str) -> Option<EvidenceKind> {
    Some(match kind {
        "changed_source" => EvidenceKind::ChangedSource,
        "caller_path" => EvidenceKind::CallerPath,
        "callee_path" => EvidenceKind::CalleePath,
        "test_behavior" => EvidenceKind::TestBehavior,
        "interface_contract" => EvidenceKind::InterfaceContract,
        "configuration" => EvidenceKind::Configuration,
        "database_schema" => EvidenceKind::DatabaseSchema,
        "repository_convention" => EvidenceKind::RepositoryConvention,
        "deterministic_finding" => EvidenceKind::StaticAnalysisResult,
        _ => return None,
    })
}

/// Maps a model category onto the finding taxonomy. Security reviewer categories (REV-S-001)
/// are all `security`; anything unknown falls back to the reviewer's own category.
pub fn category(model: &str, reviewer: ReviewerKind) -> FindingCategory {
    match model {
        "correctness" => FindingCategory::Correctness,
        "security" => FindingCategory::Security,
        "tests" => FindingCategory::Testing,
        "architecture" => FindingCategory::Architecture,
        "performance" => FindingCategory::Performance,
        "maintainability" => FindingCategory::Maintainability,
        _ if crate::security::SecurityCategory::parse(model).is_some() => FindingCategory::Security,
        _ => match reviewer {
            ReviewerKind::Security => FindingCategory::Security,
            ReviewerKind::Test => FindingCategory::Testing,
            ReviewerKind::Architecture => FindingCategory::Architecture,
            ReviewerKind::Performance => FindingCategory::Performance,
            ReviewerKind::Maintainability => FindingCategory::Maintainability,
            ReviewerKind::Correctness => FindingCategory::Correctness,
        },
    }
}

fn side_of(s: &str) -> DiffSide {
    if s == "base" {
        DiffSide::Base
    } else {
        DiffSide::Head
    }
}

fn symbol_of(entry: &RefEntry) -> Option<SymbolId> {
    if entry.kind.is_symbol() {
        SymbolId::parse(&entry.target).ok()
    } else {
        None
    }
}

/// The node key of a ref: the `SymbolId` for symbols, otherwise the item id.
fn node_key(refs: &RefTable, r: &str) -> Option<String> {
    refs.get(r).map(|e| e.target.clone())
}

/// Normalises one raw candidate.
pub fn normalize(
    item: &RawItem,
    refs: &RefTable,
    reviewer: ReviewerKind,
) -> Result<NormalizedCandidate, Box<Rejection>> {
    let raw = &item.candidate;
    let raw_output_hash = model_gateway::request_hash::hash_value(&item.json);
    let reject = |code: RejectionCode, detail: &str| {
        Box::new(Rejection {
            code,
            detail: detail.to_owned(),
            ordinal: item.ordinal,
            raw_output_hash: raw_output_hash.clone(),
            raw_json: item.json.clone(),
        })
    };
    let mut notes: Vec<String> = Vec::new();

    // 1. Anchor.
    let entry = refs.get(&raw.anchor.ref_).ok_or_else(|| {
        reject(
            RejectionCode::UnresolvableReference,
            "anchor ref not in the ref table",
        )
    })?;
    if !entry.kind.is_symbol() {
        return Err(reject(
            RejectionCode::AnchorNotSymbol,
            "anchor ref is not a symbol",
        ));
    }
    let anchor_symbol = symbol_of(entry).ok_or_else(|| {
        reject(
            RejectionCode::UnresolvableReference,
            "anchor symbol id is not canonical",
        )
    })?;
    let side = side_of(&raw.anchor.side);
    let range = match side {
        DiffSide::Head => entry.range,
        DiffSide::Base => entry.base_range,
    }
    .ok_or_else(|| {
        reject(
            RejectionCode::LineOutOfRange,
            "the anchor symbol has no range on that side",
        )
    })?;
    let (mut start, mut end) = (raw.anchor.start_line, raw.anchor.end_line);
    if start == 0 || start > end {
        return Err(reject(
            RejectionCode::LineOutOfRange,
            "invalid anchor line range",
        ));
    }
    let intersects = start <= range[1] && end >= range[0];
    if !intersects {
        let outside_file =
            side == DiffSide::Head && entry.file_lines.is_some_and(|n| start > n || end > n);
        if outside_file {
            return Err(reject(
                RejectionCode::LineOutOfRange,
                "anchor lines are outside the file",
            ));
        }
        start = start.clamp(range[0], range[1]);
        end = end.clamp(range[0], range[1]);
        notes.push("anchor_clamped".into());
    }
    let anchor = Anchor {
        symbol_key: SymbolKey::of(&anchor_symbol),
        symbol_id: anchor_symbol.clone(),
        path: entry.path.clone(),
        side,
        start_line: start,
        end_line: end,
    };

    // 2. Affected symbols.
    let mut affected = vec![anchor_symbol];
    let mut linked_items = Vec::new();
    for r in &raw.affected_refs {
        match refs.get(r) {
            Some(e) if e.kind.is_symbol() => match symbol_of(e) {
                Some(id) if !affected.contains(&id) => affected.push(id),
                Some(_) => {}
                None => notes.push("affected_ref_dropped".into()),
            },
            Some(_) => {
                if !linked_items.contains(r) {
                    linked_items.push(r.clone());
                }
            }
            None => notes.push("affected_ref_dropped".into()),
        }
    }

    // 3. Evidence.
    let mut evidence = Vec::new();
    for ev in &raw.evidence {
        let (Some(kind), Some(e)) = (evidence_kind(&ev.kind), refs.get(&ev.ref_)) else {
            notes.push("evidence_dropped".into());
            continue;
        };
        let lines = match (ev.start_line, ev.end_line) {
            (Some(s), Some(t)) if s >= 1 && s <= t => Some([s, t]),
            (Some(s), None) if s >= 1 => Some([s, s]),
            _ => None,
        };
        evidence.push(NormalizedEvidence {
            kind,
            ref_: ev.ref_.clone(),
            symbol_id: symbol_of(e),
            path: e.path.clone(),
            side: side_of(&ev.side),
            lines,
            quote: ev.quote.clone(),
            explanation: ev.explanation.clone(),
        });
    }
    if evidence.is_empty() {
        return Err(reject(
            RejectionCode::EvidenceMissing,
            "no evidence item resolved",
        ));
    }

    // 4. Claimed relations.
    let mut claimed_relations = Vec::new();
    for rel in &raw.claimed_relations {
        match (node_key(refs, &rel.from), node_key(refs, &rel.to)) {
            (Some(from), Some(to)) => claimed_relations.push(NormalizedRelation {
                from,
                relation: rel.relation.clone(),
                to,
            }),
            _ => notes.push("relation_dropped".into()),
        }
    }

    // 5. Predicate.
    let subject = match node_key(refs, &raw.predicate.subject) {
        Some(k) => k,
        None => {
            if RefKind::from_ref(&raw.predicate.subject).is_some() {
                notes.push("predicate_subject_unresolved".into());
            }
            raw.predicate.subject.clone()
        }
    };
    let params: BTreeMap<String, String> = raw
        .predicate
        .params
        .iter()
        .map(|p| {
            let value = if RefKind::from_ref(&p.value).is_some() {
                node_key(refs, &p.value).unwrap_or_else(|| p.value.clone())
            } else {
                p.value.clone()
            };
            (p.name.clone(), value)
        })
        .collect();

    // 6. Category and severity.
    let severity = Severity::parse(&raw.severity).unwrap_or_else(|| {
        notes.push("severity_defaulted".into());
        Severity::Medium
    });

    notes.sort();
    notes.dedup();
    Ok(NormalizedCandidate {
        ordinal: item.ordinal,
        reviewer,
        category: category(&raw.category, reviewer),
        model_category: raw.category.clone(),
        severity,
        title: raw.title.clone(),
        claim: raw.claim.clone(),
        description: raw.description.clone(),
        claim_text_normalized: claim_text::normalize(&raw.claim, refs),
        anchor,
        affected_symbols: affected,
        linked_items,
        evidence,
        claimed_relations,
        predicate: Predicate {
            kind: raw.predicate.kind.clone(),
            subject,
            params,
        },
        corrective_direction: raw.corrective_direction.clone(),
        self_confidence: raw.self_confidence,
        raw_output_hash,
        notes,
    })
}

/// Normalises every raw item; rejected candidates are returned separately, never dropped.
pub fn normalize_all(
    items: &[RawItem],
    refs: &RefTable,
    reviewer: ReviewerKind,
) -> (Vec<NormalizedCandidate>, Vec<Rejection>) {
    let mut ok = Vec::new();
    let mut rejected = Vec::new();
    for item in items {
        match normalize(item, refs, reviewer) {
            Ok(c) => ok.push(c),
            Err(r) => rejected.push(*r),
        }
    }
    (ok, rejected)
}
