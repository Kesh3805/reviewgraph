//! Deterministic matcher between produced findings and case expectations (EVAL-002).
//!
//! - `symbol_ok`: the finding's primary symbol is the expected symbol, or the expected symbol is
//!   among its affected symbols, or a lineage-equivalent symbol (renamed in the head) is.
//! - `location_ok`: same path, and `[start_line, end_line]` intersects `[line - window, line +
//!   window]` (inclusive).
//! - `category_ok`: `any`, an equal category, or a compatible one ([`super::category_compat`]).
//! - `claim_ok`: no keywords, or one lowercase keyword occurs in title + claim + description.
//! - Match: `category_ok ∧ (symbol_ok ∨ location_ok) ∧ claim_ok`; score
//!   `3·symbol_ok + 2·location_ok + min(1, keyword hits)`.
//!
//! Assignment is greedy and one-to-one over pairs sorted by score (desc), expectation priority
//! (expected > forbidden > optional), finding id, expectation id. Published findings are
//! assigned first, so a published hit is never displaced by an internal or suppressed one.
//! Extra findings that match an already matched *expected* entry are duplicates.

use std::collections::{BTreeMap, BTreeSet};

use review_core::finding::Severity;
use serde::{Deserialize, Serialize};

use super::case::{Case, Location};
use super::category_compat::{compatible, MATCHER_VERSION};

/// The lifecycle stage a produced finding ended in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "stage", content = "state", rename_all = "snake_case")]
pub enum Stage {
    Published,
    Internal,
    Suppressed(String),
}

impl Stage {
    pub fn is_published(&self) -> bool {
        matches!(self, Self::Published)
    }
}

/// A finding produced by the pipeline under evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducedFinding {
    pub id: String,
    pub stage: Stage,
    pub category: String,
    pub severity: Severity,
    pub primary_symbol: Option<String>,
    pub affected_symbols: Vec<String>,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub title: String,
    pub claim: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectationKind {
    Expected,
    Forbidden,
    Optional,
}

impl ExpectationKind {
    const fn priority(self) -> u8 {
        match self {
            Self::Expected => 0,
            Self::Forbidden => 1,
            Self::Optional => 2,
        }
    }
}

/// How a produced finding was classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClass {
    /// Matched an expected finding.
    Tp,
    /// Matched a forbidden finding.
    FpForbidden,
    /// Published and matched nothing.
    FpUnexpected,
    /// Matched an optional finding (neutral).
    Optional,
    /// Matched an expected finding that another finding already matched.
    Duplicate,
    /// Not published and matched nothing.
    Unmatched,
}

/// How an expectation was classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectationClass {
    Hit,
    HitUnderseverity,
    HitInternalOnly,
    Miss,
    /// A forbidden finding was published.
    Triggered,
    /// A forbidden finding was not published.
    Clear,
    OptionalHit,
    OptionalMiss,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingMatch {
    pub finding_id: String,
    pub stage: Stage,
    pub class: FindingClass,
    pub expectation: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectationMatch {
    pub id: String,
    pub kind: ExpectationKind,
    pub class: ExpectationClass,
    pub finding: Option<String>,
    pub must_publish: bool,
    pub symbol: Option<String>,
}

/// The matcher output for one case, serialised into run JSONL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchResult {
    pub matcher_version: String,
    pub findings: Vec<FindingMatch>,
    pub expectations: Vec<ExpectationMatch>,
}

/// Renamed symbols: old id → new id (from `symbol_lineage`, INC-*).
pub type Lineage = BTreeMap<String, String>;

struct Exp<'a> {
    id: &'a str,
    kind: ExpectationKind,
    categories: &'a [String],
    symbol: Option<&'a str>,
    location: Option<&'a Location>,
    claim_any_of: &'a [String],
    severity_at_least: Option<Severity>,
    must_publish: bool,
}

fn expectations(case: &Case) -> Vec<Exp<'_>> {
    let mut v = Vec::new();
    for e in &case.expected_findings {
        v.push(Exp {
            id: &e.id,
            kind: ExpectationKind::Expected,
            categories: &e.categories,
            symbol: e.symbol.as_deref(),
            location: e.location.as_ref(),
            claim_any_of: &e.claim_any_of,
            severity_at_least: e.severity_at_least,
            must_publish: e.must_publish,
        });
    }
    for f in &case.forbidden_findings {
        v.push(Exp {
            id: &f.id,
            kind: ExpectationKind::Forbidden,
            categories: &f.categories,
            symbol: f.symbol.as_deref(),
            location: f.location.as_ref(),
            claim_any_of: &f.claim_any_of,
            severity_at_least: None,
            must_publish: false,
        });
    }
    for o in &case.optional_findings {
        v.push(Exp {
            id: &o.id,
            kind: ExpectationKind::Optional,
            categories: &o.categories,
            symbol: o.symbol.as_deref(),
            location: o.location.as_ref(),
            claim_any_of: &[],
            severity_at_least: None,
            must_publish: false,
        });
    }
    v
}

fn symbol_ok(f: &ProducedFinding, sym: &str, lineage: &Lineage) -> bool {
    let candidates: Vec<&str> = std::iter::once(sym)
        .chain(lineage.get(sym).map(String::as_str))
        .collect();
    candidates.iter().any(|s| {
        f.primary_symbol.as_deref() == Some(*s) || f.affected_symbols.iter().any(|a| a == s)
    })
}

fn location_ok(f: &ProducedFinding, l: &Location) -> bool {
    let lo = l.line.saturating_sub(l.window);
    let hi = l.line.saturating_add(l.window);
    f.path == l.path && f.start_line <= hi && f.end_line >= lo
}

fn keyword_hits(f: &ProducedFinding, keywords: &[String]) -> usize {
    let text = format!("{} {} {}", f.title, f.claim, f.description).to_lowercase();
    keywords
        .iter()
        .filter(|k| text.contains(k.to_lowercase().as_str()))
        .count()
}

/// Score of a matching pair, or `None` when the predicate fails.
fn score(f: &ProducedFinding, e: &Exp<'_>, lineage: &Lineage) -> Option<u32> {
    let category_ok = e.categories.iter().any(|c| compatible(c, &f.category));
    let sym = e.symbol.is_some_and(|s| symbol_ok(f, s, lineage));
    let loc = e.location.is_some_and(|l| location_ok(f, l));
    let hits = keyword_hits(f, e.claim_any_of);
    let claim_ok = e.claim_any_of.is_empty() || hits > 0;
    (category_ok && (sym || loc) && claim_ok)
        .then_some(3 * u32::from(sym) + 2 * u32::from(loc) + u32::from(hits > 0))
}

/// Matches one case. Pure and deterministic: input order does not matter.
pub fn match_case(case: &Case, findings: &[ProducedFinding], lineage: &Lineage) -> MatchResult {
    let exps = expectations(case);
    let mut sorted: Vec<&ProducedFinding> = findings.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));

    // (score, priority, finding id, exp id) pairs, published first.
    let mut pairs: Vec<(bool, u32, u8, &str, &str, usize, usize)> = Vec::new();
    for (fi, f) in sorted.iter().enumerate() {
        for (ei, e) in exps.iter().enumerate() {
            if let Some(s) = score(f, e, lineage) {
                pairs.push((
                    !f.stage.is_published(),
                    s,
                    e.kind.priority(),
                    f.id.as_str(),
                    e.id,
                    fi,
                    ei,
                ));
            }
        }
    }
    pairs.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(b.3))
            .then(a.4.cmp(b.4))
    });

    let mut finding_to_exp: BTreeMap<usize, usize> = BTreeMap::new();
    let mut exp_to_finding: BTreeMap<usize, usize> = BTreeMap::new();
    for &(_, _, _, _, _, fi, ei) in &pairs {
        if finding_to_exp.contains_key(&fi) || exp_to_finding.contains_key(&ei) {
            continue;
        }
        finding_to_exp.insert(fi, ei);
        exp_to_finding.insert(ei, fi);
    }
    let mut duplicates: BTreeSet<usize> = BTreeSet::new();
    for &(_, _, _, _, _, fi, ei) in &pairs {
        if !finding_to_exp.contains_key(&fi)
            && exps[ei].kind == ExpectationKind::Expected
            && exp_to_finding.contains_key(&ei)
        {
            duplicates.insert(fi);
        }
    }

    let finding_rows = sorted
        .iter()
        .enumerate()
        .map(|(fi, f)| {
            let assigned = finding_to_exp.get(&fi).map(|ei| &exps[*ei]);
            let class = match assigned {
                Some(e) => match e.kind {
                    ExpectationKind::Expected => FindingClass::Tp,
                    ExpectationKind::Forbidden => FindingClass::FpForbidden,
                    ExpectationKind::Optional => FindingClass::Optional,
                },
                None if duplicates.contains(&fi) => FindingClass::Duplicate,
                None if f.stage.is_published() => FindingClass::FpUnexpected,
                None => FindingClass::Unmatched,
            };
            FindingMatch {
                finding_id: f.id.clone(),
                stage: f.stage.clone(),
                class,
                expectation: assigned.map(|e| e.id.to_owned()),
                title: f.title.clone(),
            }
        })
        .collect();

    let expectation_rows = exps
        .iter()
        .enumerate()
        .map(|(ei, e)| {
            let f = exp_to_finding.get(&ei).map(|fi| sorted[*fi]);
            let published = f.is_some_and(|f| f.stage.is_published());
            let class = match e.kind {
                ExpectationKind::Expected => match f {
                    None => ExpectationClass::Miss,
                    Some(_) if !published => ExpectationClass::HitInternalOnly,
                    Some(f) if e.severity_at_least.is_some_and(|min| f.severity < min) => {
                        ExpectationClass::HitUnderseverity
                    }
                    Some(_) => ExpectationClass::Hit,
                },
                ExpectationKind::Forbidden if published => ExpectationClass::Triggered,
                ExpectationKind::Forbidden => ExpectationClass::Clear,
                ExpectationKind::Optional if f.is_some() => ExpectationClass::OptionalHit,
                ExpectationKind::Optional => ExpectationClass::OptionalMiss,
            };
            ExpectationMatch {
                id: e.id.to_owned(),
                kind: e.kind,
                class,
                finding: f.map(|f| f.id.clone()),
                must_publish: e.must_publish,
                symbol: e.symbol.map(str::to_owned),
            }
        })
        .collect();

    MatchResult {
        matcher_version: MATCHER_VERSION.to_owned(),
        findings: finding_rows,
        expectations: expectation_rows,
    }
}
