//! Layer and module boundary check (REV-A-002, part 1).
//!
//! Every **new** `IMPORTS`/`DEPENDS_ON`/`CALLS` edge whose source lies in a file the pull request
//! changed is checked against the forbidden layer pairs: the explicit
//! `rules.forbidden_dependencies` and the enforceable `layer_dependency:*` conventions of the
//! profile. An edge that already exists in the base graph is never reported (INV-009).

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use codegraph::{Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, NodeRef};
use review_core::evidence::EvidenceKind;

use super::{DeterministicFinding, PrecheckEvidence, PrecheckOutcome, SeverityHint};
use crate::config::{ReviewConfigV1, RuleSeverity};
use crate::infer::roles::Role;
use crate::model::RepositoryProfile;
use crate::policy::change::{ChangeSet, GraphPair};
use crate::policy::evidence::{deterministic, location, symbol_ref};
use crate::policy::layers::LayerMap;

pub const RULE_ID: &str = "architecture.forbidden_dependency";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairSource {
    Explicit,
    Convention,
}

/// A forbidden `from → to` layer dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForbiddenPair {
    /// The config rule id or the convention id.
    pub id: String,
    pub from: String,
    pub to: String,
    pub reason: String,
    pub fix: Option<String>,
    pub severity: RuleSeverity,
    pub source: PairSource,
}

/// Explicit rules first, then the enforceable layer-dependency conventions not already covered.
pub fn forbidden_pairs(
    config: &ReviewConfigV1,
    profile: Option<&RepositoryProfile>,
) -> Vec<ForbiddenPair> {
    let mut out: Vec<ForbiddenPair> = config
        .rules
        .forbidden_dependencies
        .iter()
        .map(|r| ForbiddenPair {
            id: r.id.clone(),
            from: r.from.clone(),
            to: r.to.clone(),
            reason: r.reason.clone(),
            fix: r.fix.clone(),
            severity: r.severity,
            source: PairSource::Explicit,
        })
        .collect();
    let Some(profile) = profile else {
        return out;
    };
    for convention in profile.conventions.iter().filter(|c| c.enforceable) {
        let Some(pair) = convention.id.strip_prefix("layer_dependency:") else {
            continue;
        };
        let Some((from, to)) = pair.split_once("->") else {
            continue;
        };
        let covered = out.iter().any(|p| {
            Role::from_layer_name(&p.from).map(Role::as_str) == Some(from)
                && Role::from_layer_name(&p.to).map(Role::as_str) == Some(to)
        });
        if !covered {
            out.push(ForbiddenPair {
                id: convention.id.clone(),
                from: from.to_owned(),
                to: to.to_owned(),
                reason: format!(
                    "{} (convention: {} of {} samples conform, confidence {:.2})",
                    convention.rule,
                    convention.samples - convention.violations,
                    convention.samples,
                    convention.confidence
                ),
                fix: None,
                severity: RuleSeverity::Medium,
                source: PairSource::Convention,
            });
        }
    }
    out
}

/// The edge kinds a boundary check looks at.
pub fn boundary_kinds() -> EdgeKindSet {
    EdgeKindSet::of(EdgeKind::Imports)
        .union(EdgeKindSet::of(EdgeKind::DependsOn))
        .union(EdgeKindSet::of(EdgeKind::Calls))
}

/// A dependency edge the pull request adds.
#[derive(Debug, Clone, Copy)]
pub struct NewEdge<'g> {
    pub kind: EdgeKind,
    pub source: NodeRef<'g>,
    pub target: NodeRef<'g>,
    pub path: &'g str,
    pub line: u32,
}

fn edge_in_base(base: &dyn GraphQuery, edge: &NewEdge<'_>) -> bool {
    let mut found = false;
    base.for_each_edge(
        edge.source.key,
        Direction::Out,
        &EdgeFilter::kinds(EdgeKindSet::of(edge.kind)),
        &mut |e| {
            if e.target == edge.target.key {
                found = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        },
    );
    found
}

/// Dependency edges of `kinds` whose source is in a changed file and that the base lacks.
pub fn new_edges<'g>(
    change: &ChangeSet,
    head: &'g dyn GraphQuery,
    base: &dyn GraphQuery,
    kinds: EdgeKindSet,
) -> Vec<NewEdge<'g>> {
    let filter = EdgeFilter::kinds(kinds);
    let mut out = Vec::new();
    let files: BTreeSet<&str> = change.live_files().collect();
    for path in &files {
        head.nodes_in_file(path, &mut |source| {
            head.for_each_edge(source.key, Direction::Out, &filter, &mut |edge| {
                if let Some(target) = head.node(edge.target) {
                    let (path, line) = match edge.location {
                        Some((p, line, _)) => (p, line),
                        None => (
                            source.file.unwrap_or_default(),
                            source.range.map_or(1, |r| r.start.line),
                        ),
                    };
                    let candidate = NewEdge {
                        kind: edge.kind,
                        source,
                        target,
                        path,
                        line,
                    };
                    if !edge_in_base(base, &candidate) {
                        out.push(candidate);
                    }
                }
                ControlFlow::Continue(())
            });
        });
    }
    out
}

fn severity_hint(severity: RuleSeverity) -> SeverityHint {
    match severity {
        RuleSeverity::Critical | RuleSeverity::High => SeverityHint::High,
        RuleSeverity::Medium => SeverityHint::Medium,
        RuleSeverity::Low | RuleSeverity::Info => SeverityHint::Low,
    }
}

/// Builds the finding for one edge that crosses a forbidden pair.
pub fn boundary_finding(edge: &NewEdge<'_>, pair: &ForbiddenPair) -> DeterministicFinding {
    let message = format!(
        "`{}` ({}) now depends on `{}` ({}) through {}; {} forbids it: {}",
        edge.source.qualified_name,
        pair.from,
        edge.target.qualified_name,
        pair.to,
        edge.kind.as_str(),
        pair.id,
        pair.reason
    );
    let symbols: Vec<_> = [
        symbol_ref(edge.source.id, edge.source.qualified_name),
        symbol_ref(edge.target.id, edge.target.qualified_name),
    ]
    .into_iter()
    .flatten()
    .collect();
    let evidence = deterministic(
        "precheck:boundaries",
        EvidenceKind::StaticAnalysisResult,
        &message,
        location(edge.path, edge.line, edge.line),
        symbols,
    )
    .into_iter()
    .collect();
    DeterministicFinding {
        rule_id: RULE_ID.to_owned(),
        symbol_key: Some(edge.source.id.to_owned()),
        path: edge.path.to_owned(),
        line: edge.line,
        message,
        severity_hint: severity_hint(pair.severity),
        details: vec![PrecheckEvidence::Edge {
            kind: edge.kind.as_str().to_owned(),
            from: edge.source.id.to_owned(),
            to: edge.target.id.to_owned(),
            path: edge.path.to_owned(),
            line: edge.line,
        }],
        evidence,
        layers: Some((pair.from.clone(), pair.to.clone())),
        basis_id: Some(pair.id.clone()),
    }
}

/// Checks every new dependency edge of the change against `pairs`.
pub fn check_boundaries(
    change: &ChangeSet,
    graphs: &GraphPair<'_>,
    layers: &LayerMap,
    pairs: &[ForbiddenPair],
) -> PrecheckOutcome {
    if layers.is_empty() {
        return PrecheckOutcome::not_executed("no layers defined or inferred");
    }
    let usable: Vec<&ForbiddenPair> = pairs
        .iter()
        .filter(|p| layers.defines(&p.from) && layers.defines(&p.to))
        .collect();
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for edge in new_edges(change, graphs.head, graphs.base, boundary_kinds()) {
        for pair in &usable {
            if layers.in_layer(&pair.from, &edge.source) && layers.in_layer(&pair.to, &edge.target) {
                let key = (pair.id.clone(), edge.source.id.to_owned(), edge.target.id.to_owned());
                if seen.insert(key) {
                    findings.push(boundary_finding(&edge, pair));
                }
            }
        }
    }
    findings.sort_by(|a, b| (&a.path, a.line, &a.basis_id).cmp(&(&b.path, b.line, &b.basis_id)));
    crate::metrics::deterministic_findings(RULE_ID, findings.len());
    PrecheckOutcome::executed(findings, false)
}
