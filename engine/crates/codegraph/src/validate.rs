//! The consistency validator (CG-012).
//!
//! Two jobs. First, cheap production validation: a snapshot that fails here can be marked
//! `inconsistent` and rebuilt (ADR-004, PRD §24). Second, and more importantly, it is the oracle
//! the incremental stages are tested against — "the head graph is valid" is a necessary but not
//! sufficient condition, and [`crate::compare`] supplies the sufficient one.
//!
//! # Severity
//!
//! Errors mean the graph cannot be trusted: a dangling edge, a duplicate key, an asymmetric
//! reverse index, a schema mismatch or a confidence outside `0..=1000`. Warnings mean something is
//! unusual but explicable: an endpoint-kind combination outside the CG-002 matrix (language
//! extensions must not fail a build), a synthetic node nothing points at, or a node outside its
//! file's declared range.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::ControlFlow;

use crate::delta::GraphDelta;
use crate::edge::EdgeIdentity;
use crate::edge_kind::Direction;
use crate::graph::Graph;
use crate::node_id::NodeKey;
use crate::node_kind::NodeKind;
use crate::query::{EdgeFilter, GraphQuery};
use crate::schema_rules;

/// What kind of problem an issue is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IssueCode {
    /// An edge endpoint is not a node.
    DanglingEdge,
    /// Two node rows carry the same key.
    DuplicateNodeKey,
    /// A forward edge is missing from, or doubled in, the reverse index.
    ReverseIndexAsymmetry,
    /// The graph was built against another schema version.
    SchemaVersionMismatch,
    /// The endpoint kinds are outside the CG-002 matrix. Warning only.
    KindRuleViolation,
    /// A synthetic-only node has no incident edge. Warning only.
    OrphanSyntheticNode,
    /// A node sits outside the node range its file declares.
    NodeOutsideFileRange,
    /// An edge confidence is outside `0..=1000`.
    ConfidenceOutOfRange,
    /// An added edge in a delta points at a node neither the base nor the delta has.
    DanglingAddedEdge,
    /// A base edge survives although one of its endpoints was removed.
    SurvivingEdgeToRemovedNode,
    /// A `Deleted` file contributes nodes.
    DeletedFileContributesNodes,
}

impl IssueCode {
    pub const ALL: [IssueCode; 11] = [
        IssueCode::DanglingEdge,
        IssueCode::DuplicateNodeKey,
        IssueCode::ReverseIndexAsymmetry,
        IssueCode::SchemaVersionMismatch,
        IssueCode::KindRuleViolation,
        IssueCode::OrphanSyntheticNode,
        IssueCode::NodeOutsideFileRange,
        IssueCode::ConfidenceOutOfRange,
        IssueCode::DanglingAddedEdge,
        IssueCode::SurvivingEdgeToRemovedNode,
        IssueCode::DeletedFileContributesNodes,
    ];

    /// Stable lower-snake label, used by the `graph_consistency_issues_total{code}` counter and by
    /// the API payload.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DanglingEdge => "dangling_edge",
            Self::DuplicateNodeKey => "duplicate_node_key",
            Self::ReverseIndexAsymmetry => "reverse_index_asymmetry",
            Self::SchemaVersionMismatch => "schema_version_mismatch",
            Self::KindRuleViolation => "kind_rule_violation",
            Self::OrphanSyntheticNode => "orphan_synthetic_node",
            Self::NodeOutsideFileRange => "node_outside_file_range",
            Self::ConfidenceOutOfRange => "confidence_out_of_range",
            Self::DanglingAddedEdge => "dangling_added_edge",
            Self::SurvivingEdgeToRemovedNode => "surviving_edge_to_removed_node",
            Self::DeletedFileContributesNodes => "deleted_file_contributes_nodes",
        }
    }

    /// The severity this code is reported at.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::KindRuleViolation | Self::OrphanSyntheticNode => Severity::Warning,
            Self::DanglingEdge
            | Self::DuplicateNodeKey
            | Self::ReverseIndexAsymmetry
            | Self::SchemaVersionMismatch
            | Self::NodeOutsideFileRange
            | Self::ConfidenceOutOfRange
            | Self::DanglingAddedEdge
            | Self::SurvivingEdgeToRemovedNode
            | Self::DeletedFileContributesNodes => Severity::Error,
        }
    }
}

impl fmt::Display for IssueCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errors fail a snapshot; warnings are diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One problem, addressed by key or edge identity rather than by index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub code: IssueCode,
    pub severity: Severity,
    /// The node key (32 hex characters) or the edge identity that the issue is about.
    pub subject: String,
    /// Human-readable explanation. Keys, ids and paths only — never source text, so the report is
    /// safe to print in CI.
    pub detail: String,
}

impl ValidationIssue {
    fn new(code: IssueCode, subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code,
            severity: code.severity(),
            subject: subject.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.code, self.subject)?;
        if !self.detail.is_empty() {
            write!(f, " — {}", self.detail)?;
        }
        Ok(())
    }
}

/// What [`validate`] found, plus what it looked at so a caller can judge coverage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub errors: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
    pub checked_nodes: u64,
    pub checked_edges: u64,
}

impl ValidationReport {
    /// True when the graph can be trusted.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Every issue, errors first then warnings, in report order.
    pub fn issues(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.errors.iter().chain(self.warnings.iter())
    }

    /// A bounded, human-readable rendering. Truncates explicitly rather than silently (master-plan
    /// principle 4).
    #[must_use]
    pub fn render(&self, max_items: usize) -> String {
        let mut out = format!(
            "{} error(s), {} warning(s) over {} node(s) and {} edge(s)",
            self.errors.len(),
            self.warnings.len(),
            self.checked_nodes,
            self.checked_edges
        );
        let shown = max_items.min(self.errors.len() + self.warnings.len());
        for issue in self.issues().take(shown) {
            out.push('\n');
            out.push_str(&issue.to_string());
        }
        let hidden = (self.errors.len() + self.warnings.len()) - shown;
        if hidden > 0 {
            out.push_str(&format!("\n… {hidden} more issue(s) not shown"));
        }
        out
    }
}

/// Checks a built graph. `O(V + E)`.
///
/// Never fails: it always returns a report, so a caller can log it, count it and decide.
#[must_use]
pub fn validate(graph: &Graph, expected_schema: u32) -> ValidationReport {
    let mut report = ValidationReport {
        checked_nodes: graph.nodes().len() as u64,
        checked_edges: graph.edges().len() as u64,
        ..ValidationReport::default()
    };

    if graph.schema_version() != expected_schema {
        report.errors.push(ValidationIssue::new(
            IssueCode::SchemaVersionMismatch,
            format!("graph/{}", graph.schema_version()),
            format!("this build expects schema {expected_schema}"),
        ));
    }

    // Every edge endpoint must resolve, every key must be unique, every file range must cover
    // exactly the nodes that name it.
    let mut seen_keys: HashSet<NodeKey> = HashSet::with_capacity(graph.nodes().len());
    for (raw, node) in graph.nodes().iter().enumerate() {
        if !seen_keys.insert(node.key) {
            report.errors.push(ValidationIssue::new(
                IssueCode::DuplicateNodeKey,
                node.key.to_string(),
                "the key appears on more than one node row",
            ));
        }
        if let Some(entry) = node.file.and_then(|ix| graph.file(ix)) {
            let range = entry.nodes.clone();
            let position = u32::try_from(raw).unwrap_or(u32::MAX);
            if position < range.start || position >= range.end {
                report.errors.push(ValidationIssue::new(
                    IssueCode::NodeOutsideFileRange,
                    node.key.to_string(),
                    format!(
                        "{} claims nodes {}..{} but the node sits at {position}",
                        graph.str(entry.path),
                        range.start,
                        range.end
                    ),
                ));
            }
        }
    }

    // Reverse-index symmetry: every edge appears exactly once in the reverse CSR of its target
    // and exactly once in the forward CSR of its source. Counted per identity rather than per
    // index, so a doubled entry is caught as well as a missing one.
    let mut forward_counts: HashMap<EdgeIdentity, u32> =
        HashMap::with_capacity(graph.edges().len());
    let mut reverse_counts: HashMap<EdgeIdentity, u32> =
        HashMap::with_capacity(graph.edges().len());
    for (raw, edge) in graph.edges().iter().enumerate() {
        let (Some(source), Some(target)) = (graph.node(edge.source), graph.node(edge.target))
        else {
            report.errors.push(ValidationIssue::new(
                IssueCode::DanglingEdge,
                format!("edge#{}", edge.source.get()),
                "an edge endpoint is not in the node table",
            ));
            continue;
        };
        let identity = EdgeIdentity {
            source: source.key,
            kind: edge.kind,
            target: target.key,
        };
        *forward_counts.entry(identity).or_insert(0) += 1;

        let own = crate::graph::EdgeIx::new(u32::try_from(raw).unwrap_or(u32::MAX));
        let in_source = graph.out_edges(edge.source).contains(&own);
        let in_target = graph.in_edges(edge.target).contains(&own);
        if in_source && in_target {
            *reverse_counts.entry(identity).or_insert(0) += 1;
        } else {
            report.errors.push(ValidationIssue::new(
                IssueCode::ReverseIndexAsymmetry,
                identity.to_string(),
                format!("forward slice: {in_source}, reverse slice: {in_target}"),
            ));
        }

        if edge.confidence.as_permille() > 1000 {
            report.errors.push(ValidationIssue::new(
                IssueCode::ConfidenceOutOfRange,
                identity.to_string(),
                format!(
                    "confidence {}‰ is out of range",
                    edge.confidence.as_permille()
                ),
            ));
        }
    }
    for (identity, forward) in &forward_counts {
        if *forward > 1 {
            report.errors.push(ValidationIssue::new(
                IssueCode::DuplicateNodeKey,
                identity.to_string(),
                format!("the edge identity appears {forward} times"),
            ));
        }
    }

    // Endpoint-kind rules and orphan synthetic nodes.
    let mut incident: HashSet<NodeKey> = HashSet::with_capacity(graph.nodes().len() * 2);
    for edge in graph.edges() {
        let (Some(source), Some(target)) = (graph.node(edge.source), graph.node(edge.target))
        else {
            continue;
        };
        incident.insert(source.key);
        incident.insert(target.key);
        if !schema_rules::is_allowed(edge.kind, source.kind.category(), target.kind.category()) {
            report.warnings.push(ValidationIssue::new(
                IssueCode::KindRuleViolation,
                format!("{} -{}-> {}", source.key, edge.kind, target.key),
                format!(
                    "{} -> {} is outside the CG-002 endpoint matrix",
                    source.kind, target.kind
                ),
            ));
        }
    }
    for node in graph.nodes() {
        if node.kind.is_synthetic_only()
            && !incident.contains(&node.key)
            && node.kind != NodeKind::Repository
        {
            report.warnings.push(ValidationIssue::new(
                IssueCode::OrphanSyntheticNode,
                node.key.to_string(),
                format!(
                    "{} has no incident edge, so it is not evidence of anything",
                    node.kind
                ),
            ));
        }
    }

    report
        .errors
        .sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.subject.cmp(&b.subject)));
    report
        .warnings
        .sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.subject.cmp(&b.subject)));
    report
}

/// The local validity check for one delta against its base. `O(|Δ| · deg)`.
///
/// * every added edge's endpoints must exist in the base or in the delta,
/// * no base edge may survive that points at a removed node,
/// * a `Deleted` file may not contribute nodes.
///
/// Never fails: it returns the issues it found.
#[must_use]
pub fn validate_delta_local(base: &Graph, delta: &GraphDelta) -> Vec<ValidationIssue> {
    let mut out = Vec::new();
    let added_keys: HashSet<NodeKey> = delta.nodes_added.iter().map(|n| n.id.key()).collect();
    let removed: HashSet<NodeKey> = delta.nodes_removed.iter().copied().collect();
    let known = |key: &NodeKey| {
        added_keys.contains(key) || (base.index_of_key(key).is_some() && !removed.contains(key))
    };

    for edge in &delta.edges_added {
        if !known(&edge.source) || !known(&edge.target) {
            out.push(ValidationIssue::new(
                IssueCode::DanglingAddedEdge,
                edge.identity().to_string(),
                format!(
                    "source known: {}, target known: {}",
                    known(&edge.source),
                    known(&edge.target)
                ),
            ));
        }
    }

    for key in &delta.nodes_removed {
        let Some(index) = base.index_of_key(key) else {
            continue;
        };
        let node = base.node(index);
        let Some(node) = node else { continue };
        let removed_edge =
            |dir: Direction, visit: &mut dyn FnMut(crate::EdgeRef<'_>) -> ControlFlow<()>| {
                base.for_each_edge(node.key, dir, &EdgeFilter::ALL, visit);
            };
        let mut broken: Vec<String> = Vec::new();
        for dir in [Direction::Out, Direction::In] {
            removed_edge(dir, &mut |edge| {
                let other = if edge.source == node.key {
                    edge.target
                } else {
                    edge.source
                };
                // When the other endpoint goes too the edge vanishes with it, so nothing breaks.
                if removed.contains(&other) {
                    return ControlFlow::Continue(());
                }
                // Survives only when the delta tombstones it or replaces it.
                let identity = EdgeIdentity {
                    source: edge.source,
                    kind: edge.kind,
                    target: edge.target,
                };
                let tombstoned = delta.edges_removed.contains(&identity);
                let replaced = delta
                    .edges_added
                    .iter()
                    .any(|candidate| candidate.identity() == identity);
                if !tombstoned && !replaced {
                    broken.push(identity.to_string());
                }
                ControlFlow::Continue(())
            });
        }
        for identity in broken {
            out.push(ValidationIssue::new(
                IssueCode::SurvivingEdgeToRemovedNode,
                identity,
                format!("node {key} was removed but this edge was not"),
            ));
        }
    }

    for change in &delta.files {
        if !matches!(change.change, crate::delta::FileChangeKind::Deleted) {
            continue;
        }
        for node in &delta.nodes_added {
            if node.file.as_ref() == Some(&change.path) {
                out.push(ValidationIssue::new(
                    IssueCode::DeletedFileContributesNodes,
                    node.id.key().to_string(),
                    format!(
                        "{} is deleted but contributes this node",
                        change.path.as_str()
                    ),
                ));
            }
        }
    }

    out.sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.subject.cmp(&b.subject)));
    out.dedup();
    out
}
