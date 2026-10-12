//! Deterministic matcher between produced findings and case expectations (EVAL-002).
//!
//! Pure function: the same inputs always give the same assignment (ties are broken by ids).

use std::collections::{BTreeMap, BTreeSet};

use review_core::finding::Severity;
use serde::{Deserialize, Serialize};

use super::case::{Case, Expectation};
use super::category_compat::categories_compatible;

/// Where a produced finding ended up in the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "stage", content = "state")]
pub enum Stage {
    Published,
    Internal,
    Suppressed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducedFinding {
    pub id: String,
    pub stage: Stage,
    pub category: String,
    pub severity: Severity,
    pub primary_symbol: Option<String>,
    #[serde(default)]
    pub affected_symbols: Vec<String>,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub title: String,
    pub claim: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClass {
    Tp,
    FpForbidden,
    FpUnexpected,
    Optional,
    Duplicate,
    /// Unmatched and not published: not a false positive.
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectationClass {
    Hit,
    HitUnderseverity,
    HitInternalOnly,
    Miss,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MatchResult {
    pub matcher_version: u32,
    /// `(finding id, class, matched expectation id)` sorted by finding id.
    pub findings: Vec<(String, FindingClass, Option<String>)>,
    /// `(expectation id, class, matching finding id)` for expected findings, sorted by id.
    pub expected: Vec<(String, ExpectationClass, Option<String>)>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Expected,
    Forbidden,
    Optional,
}

struct Pair<'a> {
    finding: &'a ProducedFinding,
    exp: &'a Expectation,
    kind: Kind,
    score: u32,
}

fn symbol_ok(f: &ProducedFinding, exp: &Expectation, renames: &BTreeMap<String, String>) -> bool {
    let Some(sym) = &exp.symbol else {
        return false;
    };
    let mapped = renames.get(sym);
    let matches = |s: &String| s == sym || mapped == Some(s);
    f.primary_symbol.as_ref().is_some_and(matches) || f.affected_symbols.iter().any(matches)
}

fn location_ok(f: &ProducedFinding, exp: &Expectation) -> bool {
    let Some(loc) = &exp.location else {
        return false;
    };
    if f.path != loc.path {
        return false;
    }
    let lo = loc.line.saturating_sub(loc.window);
    let hi = loc.line.saturating_add(loc.window);
    f.start_line <= hi && f.end_line >= lo
}

fn category_ok(f: &ProducedFinding, exp: &Expectation) -> bool {
    exp.categories
        .iter()
        .any(|c| categories_compatible(c, &f.category))
}

fn claim_hits(f: &ProducedFinding, exp: &Expectation) -> usize {
    let hay = format!("{} {} {}", f.title, f.claim, f.description).to_lowercase();
    exp.claim_any_of
        .iter()
        .filter(|k| hay.contains(&k.to_lowercase()))
        .count()
}

fn pair_for<'a>(
    f: &'a ProducedFinding,
    exp: &'a Expectation,
    kind: Kind,
    renames: &BTreeMap<String, String>,
) -> Option<Pair<'a>> {
    let sym = symbol_ok(f, exp, renames);
    let loc = location_ok(f, exp);
    let hits = claim_hits(f, exp);
    let claim = exp.claim_any_of.is_empty() || hits > 0;
    if !(category_ok(f, exp) && (sym || loc) && claim) {
        return None;
    }
    Some(Pair {
        finding: f,
        exp,
        kind,
        score: 3 * u32::from(sym) + 2 * u32::from(loc) + hits.min(1) as u32,
    })
}

/// Matches `findings` against the case expectations. `renames` maps expected symbol ids to
/// lineage-equivalent head symbol ids.
pub fn match_case(
    case: &Case,
    findings: &[ProducedFinding],
    renames: &BTreeMap<String, String>,
) -> MatchResult {
    let mut pairs: Vec<Pair<'_>> = Vec::new();
    for f in findings {
        let groups = [
            (&case.expected_findings, Kind::Expected),
            (&case.forbidden_findings, Kind::Forbidden),
            (&case.optional_findings, Kind::Optional),
        ];
        for (list, kind) in groups {
            for exp in list {
                if let Some(p) = pair_for(f, exp, kind, renames) {
                    pairs.push(p);
                }
            }
        }
    }
    pairs.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.kind.cmp(&b.kind))
            .then_with(|| a.finding.id.cmp(&b.finding.id))
            .then_with(|| a.exp.id.cmp(&b.exp.id))
    });

    let mut used_findings: BTreeMap<&str, (&Expectation, Kind)> = BTreeMap::new();
    let mut used_exps: BTreeMap<&str, &ProducedFinding> = BTreeMap::new();
    for p in &pairs {
        if used_findings.contains_key(p.finding.id.as_str())
            || used_exps.contains_key(p.exp.id.as_str())
        {
            continue;
        }
        used_findings.insert(p.finding.id.as_str(), (p.exp, p.kind));
        used_exps.insert(p.exp.id.as_str(), p.finding);
    }

    // Unassigned findings that also satisfy a matched expected entry are duplicates.
    let matched_expected: BTreeSet<&str> = case
        .expected_findings
        .iter()
        .filter(|e| used_exps.contains_key(e.id.as_str()))
        .map(|e| e.id.as_str())
        .collect();
    let duplicates: BTreeSet<&str> = pairs
        .iter()
        .filter(|p| {
            p.kind == Kind::Expected
                && matched_expected.contains(p.exp.id.as_str())
                && !used_findings.contains_key(p.finding.id.as_str())
        })
        .map(|p| p.finding.id.as_str())
        .collect();

    let mut out = MatchResult {
        matcher_version: super::category_compat::MATCHER_VERSION,
        ..MatchResult::default()
    };
    for f in findings {
        let published = f.stage == Stage::Published;
        let (class, exp_id) = match used_findings.get(f.id.as_str()) {
            Some((exp, Kind::Expected)) => (FindingClass::Tp, Some(exp.id.clone())),
            Some((exp, Kind::Forbidden)) if published => {
                (FindingClass::FpForbidden, Some(exp.id.clone()))
            }
            Some((exp, Kind::Forbidden)) => (FindingClass::Ignored, Some(exp.id.clone())),
            Some((exp, Kind::Optional)) => (FindingClass::Optional, Some(exp.id.clone())),
            None if duplicates.contains(f.id.as_str()) => (FindingClass::Duplicate, None),
            None if published => (FindingClass::FpUnexpected, None),
            None => (FindingClass::Ignored, None),
        };
        out.findings.push((f.id.clone(), class, exp_id));
    }
    out.findings.sort_by(|a, b| a.0.cmp(&b.0));

    for exp in &case.expected_findings {
        let entry = match used_exps.get(exp.id.as_str()) {
            None => (exp.id.clone(), ExpectationClass::Miss, None),
            Some(f) => {
                let class = if f.stage != Stage::Published {
                    ExpectationClass::HitInternalOnly
                } else if exp.severity_at_least.is_some_and(|min| f.severity < min) {
                    ExpectationClass::HitUnderseverity
                } else {
                    ExpectationClass::Hit
                };
                (exp.id.clone(), class, Some(f.id.clone()))
            }
        };
        out.expected.push(entry);
    }
    out.expected.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::eval::case::parse_case;

    fn case(extra: &str) -> Case {
        parse_case(&format!(
            "id: corr-001-x\nversion: 1\ntitle: t\nclasses: [correctness_regression]\n\
             language: typescript\npr: {{ title: p, base_branch: main }}\n\
             required_reviewers: [correctness]\n{extra}"
        ))
        .unwrap()
    }

    fn finding(id: &str, stage: Stage, line: u32) -> ProducedFinding {
        ProducedFinding {
            id: id.into(),
            stage,
            category: "correctness".into(),
            severity: Severity::High,
            primary_symbol: None,
            affected_symbols: vec![],
            path: "src/a.ts".into(),
            start_line: line,
            end_line: line,
            title: "Missing await".into(),
            claim: "promise not awaited".into(),
            description: String::new(),
        }
    }

    const LOC: &str = "expected_findings:\n  - id: E1\n    categories: [correctness]\n    \
                       location: { path: src/a.ts, line: 10, window: 2 }\n";

    #[test]
    fn location_window_inclusive() {
        let c = case(LOC);
        let r = match_case(&c, &[finding("f1", Stage::Published, 12)], &BTreeMap::new());
        assert_eq!(r.expected[0].1, ExpectationClass::Hit);
        let r = match_case(&c, &[finding("f1", Stage::Published, 13)], &BTreeMap::new());
        assert_eq!(r.expected[0].1, ExpectationClass::Miss);
        assert_eq!(r.findings[0].1, FindingClass::FpUnexpected);
    }

    #[test]
    fn category_any_matches_all() {
        let c = case(
            "expected_findings:\n  - id: E1\n    categories: [any]\n    \
             location: { path: src/a.ts, line: 10 }\n",
        );
        let mut f = finding("f1", Stage::Published, 10);
        f.category = "style".into();
        assert_eq!(
            match_case(&c, &[f], &BTreeMap::new()).expected[0].1,
            ExpectationClass::Hit
        );
    }

    #[test]
    fn compat_security_correctness() {
        let c = case(
            "expected_findings:\n  - id: E1\n    categories: [security]\n    \
             location: { path: src/a.ts, line: 10 }\n",
        );
        let f = finding("f1", Stage::Published, 10);
        assert_eq!(
            match_case(&c, &[f], &BTreeMap::new()).expected[0].1,
            ExpectationClass::Hit
        );
    }

    #[test]
    fn duplicate_of_matched_expected_reported() {
        let c = case(LOC);
        let r = match_case(
            &c,
            &[
                finding("f1", Stage::Published, 10),
                finding("f2", Stage::Published, 11),
            ],
            &BTreeMap::new(),
        );
        assert_eq!(r.findings[0].1, FindingClass::Tp);
        assert_eq!(r.findings[1].1, FindingClass::Duplicate);
    }

    #[test]
    fn forbidden_published_is_fp_and_suppressed_is_not() {
        let c = case(
            "forbidden_findings:\n  - id: F1\n    categories: [any]\n    \
             location: { path: src/a.ts, line: 10 }\n",
        );
        let r = match_case(&c, &[finding("f1", Stage::Published, 10)], &BTreeMap::new());
        assert_eq!(r.findings[0].1, FindingClass::FpForbidden);
        let s = Stage::Suppressed("verifier".into());
        let r = match_case(&c, &[finding("f1", s, 10)], &BTreeMap::new());
        assert_eq!(r.findings[0].1, FindingClass::Ignored);
    }

    #[test]
    fn optional_is_neutral() {
        let c = case(
            "optional_findings:\n  - id: O1\n    categories: [any]\n    \
             location: { path: src/a.ts, line: 10 }\n",
        );
        let r = match_case(&c, &[finding("f1", Stage::Published, 10)], &BTreeMap::new());
        assert_eq!(r.findings[0].1, FindingClass::Optional);
    }

    #[test]
    fn internal_only_is_reported_distinctly() {
        let c = case(LOC);
        let r = match_case(&c, &[finding("f1", Stage::Internal, 10)], &BTreeMap::new());
        assert_eq!(r.expected[0].1, ExpectationClass::HitInternalOnly);
    }

    #[test]
    fn assignment_is_deterministic_under_permutation() {
        let c = case(LOC);
        let a = finding("f1", Stage::Published, 10);
        let b = finding("f2", Stage::Published, 11);
        let r1 = match_case(&c, &[a.clone(), b.clone()], &BTreeMap::new());
        let r2 = match_case(&c, &[b, a], &BTreeMap::new());
        assert_eq!(r1.findings, r2.findings);
        assert_eq!(r1.expected, r2.expected);
    }
}
