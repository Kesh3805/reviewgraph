//! Cross-reviewer merge (DED-002): deterministic grouping of verified findings.
//!
//! Pure logic. Persistence (`finding_merge_records`) and the CAS transitions are done by the
//! pipeline store; this module has no I/O.

use std::collections::BTreeMap;

use review_core::finding::severity::Severity;

use super::normalize::NormalizedClaim;

/// Linking rule that joined two candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MergeRule {
    /// Equal DED-001 fingerprint.
    R1,
    /// Same anchor symbol, same predicate kind, evidence-range Jaccard >= 0.5.
    R2,
    /// Same anchor symbol, overlapping anchor ranges, claim-token Jaccard >= 0.6.
    R3,
}

/// Inclusive line range inside one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    fn overlaps(&self, o: &LineRange) -> bool {
        self.start <= o.end && o.start <= self.end
    }
}

/// Input row: a candidate in state `VERIFIED`.
#[derive(Debug, Clone)]
pub struct VerifiedCandidate {
    pub candidate_id: String,
    pub reviewer_kind: String,
    pub fingerprint: String,
    pub anchor_symbol_key: Option<String>,
    pub predicate_kind: Option<String>,
    pub anchor_range: Option<LineRange>,
    pub evidence_ranges: Vec<LineRange>,
    pub claim: NormalizedClaim,
    pub severity: Severity,
    pub confidence: f32,
    pub evidence_count: usize,
}

#[derive(Debug, Clone)]
pub struct MergeGroup {
    pub primary: String,
    pub duplicates: Vec<String>,
    pub rule: MergeRule,
    pub similarity: f32,
    pub confidence_before: f32,
    pub confidence_after: f32,
    pub contributing_reviewers: Vec<String>,
}

fn range_jaccard(a: &[LineRange], b: &[LineRange]) -> f32 {
    let cover = |rs: &[LineRange]| -> std::collections::BTreeSet<u32> {
        rs.iter().flat_map(|r| r.start..=r.end).collect()
    };
    let (sa, sb) = (cover(a), cover(b));
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count();
    inter as f32 / (sa.len() + sb.len() - inter) as f32
}

fn link(a: &VerifiedCandidate, b: &VerifiedCandidate) -> Option<(MergeRule, f32)> {
    if a.fingerprint == b.fingerprint {
        return Some((MergeRule::R1, 1.0));
    }
    let (sa, sb) = (a.anchor_symbol_key.as_deref()?, b.anchor_symbol_key.as_deref()?);
    if sa != sb {
        return None;
    }
    if a.predicate_kind.is_some() && a.predicate_kind == b.predicate_kind {
        let j = range_jaccard(&a.evidence_ranges, &b.evidence_ranges);
        if j >= 0.5 {
            return Some((MergeRule::R2, j));
        }
    }
    if let (Some(ra), Some(rb)) = (a.anchor_range, b.anchor_range) {
        if ra.overlaps(&rb) {
            let j = a.claim.jaccard(&b.claim);
            if j >= 0.6 {
                return Some((MergeRule::R3, j));
            }
        }
    }
    None
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// `agreement = min(1, (distinct_reviewer_kinds - 1) / 2)` (VER-009).
pub fn agreement(distinct_reviewer_kinds: usize) -> f32 {
    (distinct_reviewer_kinds.saturating_sub(1) as f32 / 2.0).min(1.0)
}

/// Group duplicates. Output is independent of input order. Singletons are omitted.
///
/// `confidence_after` is `confidence_before` raised only by the agreement delta: the caller
/// supplies the real VER-009 recompute via `recompute`.
pub fn merge_verified(
    input: &[VerifiedCandidate],
    recompute: impl Fn(&VerifiedCandidate, f32) -> f32,
) -> Vec<MergeGroup> {
    let mut items: Vec<&VerifiedCandidate> = input.iter().collect();
    items.sort_by(|a, b| (&a.fingerprint, &a.candidate_id).cmp(&(&b.fingerprint, &b.candidate_id)));
    let n = items.len();
    let mut parent: Vec<usize> = (0..n).collect();
    // Best (lowest) rule + similarity per component root pair, keyed later by group.
    let mut links: Vec<(usize, usize, MergeRule, f32)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            if let Some((rule, sim)) = link(items[i], items[j]) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[rj.max(ri)] = rj.min(ri);
                }
                links.push((i, j, rule, sim));
            }
        }
    }
    let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        comps.entry(r).or_default().push(i);
    }
    let mut out = Vec::new();
    for members in comps.values().filter(|m| m.len() > 1) {
        let mut ms: Vec<&VerifiedCandidate> = members.iter().map(|&i| items[i]).collect();
        // Total order: confidence desc, severity desc, evidence desc, id asc.
        ms.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then(b.severity.cmp(&a.severity))
                .then(b.evidence_count.cmp(&a.evidence_count))
                .then(a.candidate_id.cmp(&b.candidate_id))
        });
        let primary = ms[0];
        let (rule, similarity) = links
            .iter()
            .filter(|(i, _, _, _)| members.contains(i))
            .map(|&(_, _, r, s)| (r, s))
            .min_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)))
            .unwrap_or((MergeRule::R1, 1.0));
        let mut kinds: Vec<String> = ms.iter().map(|m| m.reviewer_kind.clone()).collect();
        kinds.sort();
        kinds.dedup();
        let after = recompute(primary, agreement(kinds.len())).max(primary.confidence);
        out.push(MergeGroup {
            primary: primary.candidate_id.clone(),
            duplicates: {
                let mut d: Vec<String> =
                    ms[1..].iter().map(|m| m.candidate_id.clone()).collect();
                d.sort();
                d
            },
            rule,
            similarity,
            confidence_before: primary.confidence,
            confidence_after: after,
            contributing_reviewers: kinds,
        });
    }
    out.sort_by(|a, b| a.primary.cmp(&b.primary));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedup::normalize_claim;

    fn c(id: &str, reviewer: &str, fp: &str, sev: Severity, conf: f32) -> VerifiedCandidate {
        VerifiedCandidate {
            candidate_id: id.into(),
            reviewer_kind: reviewer.into(),
            fingerprint: fp.into(),
            anchor_symbol_key: Some("sym".into()),
            predicate_kind: Some("auth".into()),
            anchor_range: Some(LineRange { start: 10, end: 20 }),
            evidence_ranges: vec![LineRange { start: 10, end: 20 }],
            claim: normalize_claim("authorization check removed"),
            severity: sev,
            confidence: conf,
            evidence_count: 1,
        }
    }

    fn bump(c: &VerifiedCandidate, agr: f32) -> f32 {
        c.confidence + 0.1 * agr
    }

    #[test]
    fn three_reviewers_become_one_group() {
        let v = vec![
            c("c3", "architecture", "v2:c", Severity::Medium, 0.7),
            c("c1", "security", "v2:a", Severity::High, 0.8),
            c("c2", "correctness", "v2:b", Severity::High, 0.75),
        ];
        let g = merge_verified(&v, bump);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].primary, "c1");
        assert_eq!(g[0].duplicates, vec!["c2", "c3"]);
        assert!(g[0].confidence_after > g[0].confidence_before);
    }

    #[test]
    fn independent_of_input_order() {
        let v = vec![
            c("c3", "architecture", "v2:c", Severity::Medium, 0.7),
            c("c1", "security", "v2:a", Severity::High, 0.8),
            c("c2", "correctness", "v2:b", Severity::High, 0.75),
        ];
        let base = merge_verified(&v, bump);
        let mut r = v.clone();
        r.reverse();
        let g = merge_verified(&r, bump);
        assert_eq!(base[0].primary, g[0].primary);
        assert_eq!(base[0].duplicates, g[0].duplicates);
    }

    #[test]
    fn different_symbols_not_merged() {
        let a = c("c1", "security", "v2:a", Severity::High, 0.8);
        let mut b = c("c2", "correctness", "v2:b", Severity::High, 0.8);
        b.anchor_symbol_key = Some("other".into());
        assert!(merge_verified(&[a, b], bump).is_empty());
    }

    #[test]
    fn merge_never_raises_severity_and_agreement_scale() {
        assert_eq!(agreement(1), 0.0);
        assert_eq!(agreement(2), 0.5);
        assert_eq!(agreement(5), 1.0);
    }
}
