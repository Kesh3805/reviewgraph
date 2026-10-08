//! The graph diff the incremental oracle is built on (CG-012).
//!
//! [`compare`] answers one question: *are these two graphs the same graph?* It takes any two
//! [`GraphQuery`] implementors, so a PR-head overlay can be compared against a from-scratch full
//! build without flattening the overlay first (INC-012), and it is the check
//! `GraphOverlay::flatten` exists to make checkable.
//!
//! The comparison is a sorted merge in `O(V + E)`. Both inputs are walked in canonical order —
//! nodes by key, and each node's out-edges by `(kind, other key)` — so the output is
//! deterministic and the memory cost is only the report itself.

use std::fmt;
use std::ops::ControlFlow;

use crate::edge::EdgeIdentity;
use crate::node_id::NodeKey;
use crate::query::{EdgeFilter, EdgeRef, GraphQuery, NodeRef};
use crate::Direction;

/// What to leave out of the comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareOptions {
    /// Ignore `line`/`col` on edges. Off by default: a moved call site is a real change.
    pub ignore_locations: bool,
    /// Ignore `file_versions.id`. On by default: a full and an incremental build may legitimately
    /// reference different rows holding identical content.
    pub ignore_file_version_ids: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            ignore_locations: false,
            ignore_file_version_ids: true,
        }
    }
}

impl CompareOptions {
    /// The strictest comparison: byte-for-byte identical graphs.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            ignore_locations: false,
            ignore_file_version_ids: false,
        }
    }

    /// Ignores both locations and file-version ids, for comparing two builds of the same commit
    /// reached by different routes.
    #[must_use]
    pub fn lenient() -> Self {
        Self {
            ignore_locations: true,
            ignore_file_version_ids: true,
        }
    }
}

/// The structured difference between two graphs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphDiffReport {
    pub nodes_only_a: Vec<NodeKey>,
    pub nodes_only_b: Vec<NodeKey>,
    /// Nodes present in both whose fields differ, with the differing field names.
    pub nodes_differ: Vec<(NodeKey, Vec<&'static str>)>,
    pub edges_only_a: Vec<EdgeIdentity>,
    pub edges_only_b: Vec<EdgeIdentity>,
    /// Edges present in both whose payload differs, with the differing field names.
    pub edges_differ: Vec<(EdgeIdentity, Vec<&'static str>)>,
    /// `(file, ordinal)` of an unresolved reference only one side has.
    pub unresolved_only_a: Vec<(String, u32)>,
    pub unresolved_only_b: Vec<(String, u32)>,
    /// Unresolved references both sides have, but with a different reason or name.
    pub unresolved_differ: Vec<(String, u32, Vec<&'static str>)>,
}

impl GraphDiffReport {
    /// True when the two graphs are the same under the options that produced this report.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes_only_a.is_empty()
            && self.nodes_only_b.is_empty()
            && self.nodes_differ.is_empty()
            && self.edges_only_a.is_empty()
            && self.edges_only_b.is_empty()
            && self.edges_differ.is_empty()
            && self.unresolved_only_a.is_empty()
            && self.unresolved_only_b.is_empty()
            && self.unresolved_differ.is_empty()
    }

    /// A bounded, human-readable rendering.
    ///
    /// Prints keys, ids and paths only — never source text — so it is safe for CI logs. Truncation
    /// is stated explicitly (master-plan principle 4).
    #[must_use]
    pub fn render(&self, max_items: usize) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut section = |label: &str, count: usize, items: Vec<String>| {
            if count == 0 {
                return;
            }
            lines.push(format!("{label}: {count}"));
            for line in items.iter().take(max_items) {
                lines.push(format!("  {line}"));
            }
            if count > max_items {
                lines.push(format!("  … {} more not shown", count - max_items));
            }
        };
        section(
            "node only in a",
            self.nodes_only_a.len(),
            self.nodes_only_a.iter().map(NodeKey::to_string).collect(),
        );
        section(
            "node only in b",
            self.nodes_only_b.len(),
            self.nodes_only_b.iter().map(NodeKey::to_string).collect(),
        );
        section(
            "node differs",
            self.nodes_differ.len(),
            self.nodes_differ
                .iter()
                .map(|item| format!("{} {:?}", item.0, item.1))
                .collect(),
        );
        section(
            "edge only in a",
            self.edges_only_a.len(),
            self.edges_only_a
                .iter()
                .map(EdgeIdentity::to_string)
                .collect(),
        );
        section(
            "edge only in b",
            self.edges_only_b.len(),
            self.edges_only_b
                .iter()
                .map(EdgeIdentity::to_string)
                .collect(),
        );
        section(
            "edge differs",
            self.edges_differ.len(),
            self.edges_differ
                .iter()
                .map(|item| format!("{} {:?}", item.0, item.1))
                .collect(),
        );
        section(
            "unresolved only in a",
            self.unresolved_only_a.len(),
            self.unresolved_only_a
                .iter()
                .map(|item| format!("{}#{}", item.0, item.1))
                .collect(),
        );
        section(
            "unresolved only in b",
            self.unresolved_only_b.len(),
            self.unresolved_only_b
                .iter()
                .map(|item| format!("{}#{}", item.0, item.1))
                .collect(),
        );
        section(
            "unresolved differs",
            self.unresolved_differ.len(),
            self.unresolved_differ
                .iter()
                .map(|item| format!("{}#{} {:?}", item.0, item.1, item.2))
                .collect(),
        );
        if lines.is_empty() {
            return "graphs are equal".to_owned();
        }
        lines.join("\n")
    }
}

impl fmt::Display for GraphDiffReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(50))
    }
}

/// Compares two graphs field by field.
///
/// Never fails and never panics: the return value is the whole answer.
#[must_use]
pub fn compare(
    a: &dyn GraphQuery,
    b: &dyn GraphQuery,
    options: &CompareOptions,
) -> GraphDiffReport {
    let mut report = GraphDiffReport::default();
    compare_nodes(a, b, &mut report);
    compare_edges(a, b, options, &mut report);
    compare_unresolved(a, b, &mut report);
    report.nodes_only_a.sort();
    report.nodes_only_b.sort();
    report.nodes_differ.sort();
    report.edges_only_a.sort();
    report.edges_only_b.sort();
    report.edges_differ.sort();
    report.unresolved_only_a.sort();
    report.unresolved_only_b.sort();
    report.unresolved_differ.sort();
    report
}

fn compare_nodes(a: &dyn GraphQuery, b: &dyn GraphQuery, report: &mut GraphDiffReport) {
    let left: Vec<NodeRef<'_>> = collect_nodes(a);
    let right: Vec<NodeRef<'_>> = collect_nodes(b);
    let mut left_ix = 0usize;
    let mut right_ix = 0usize;
    while left_ix < left.len() && right_ix < right.len() {
        match left[left_ix].key.cmp(&right[right_ix].key) {
            std::cmp::Ordering::Less => {
                report.nodes_only_a.push(left[left_ix].key);
                left_ix += 1;
            }
            std::cmp::Ordering::Greater => {
                report.nodes_only_b.push(right[right_ix].key);
                right_ix += 1;
            }
            std::cmp::Ordering::Equal => {
                let fields = node_field_diff(left[left_ix], &right[right_ix]);
                if !fields.is_empty() {
                    report.nodes_differ.push((left[left_ix].key, fields));
                }
                left_ix += 1;
                right_ix += 1;
            }
        }
    }
    report
        .nodes_only_a
        .extend(left[left_ix..].iter().map(|node| node.key));
    report
        .nodes_only_b
        .extend(right[right_ix..].iter().map(|node| node.key));
}

fn collect_nodes(graph: &dyn GraphQuery) -> Vec<NodeRef<'_>> {
    let mut out: Vec<NodeRef<'_>> = Vec::with_capacity(graph.node_count());
    graph.for_each_node(&mut |node| out.push(node));
    out.sort_by_key(|a| a.key);
    out
}

/// The node fields that differ, in a fixed order so the report is deterministic.
fn node_field_diff(left: NodeRef<'_>, right: &NodeRef<'_>) -> Vec<&'static str> {
    let mut out = Vec::new();
    if left.kind != right.kind {
        out.push("kind");
    }
    if left.id != right.id {
        out.push("id");
    }
    if left.name != right.name {
        out.push("name");
    }
    if left.qualified_name != right.qualified_name {
        out.push("qualified_name");
    }
    if left.file != right.file {
        out.push("file");
    }
    if left.range != right.range {
        out.push("range");
    }
    if left.attrs.visibility != right.attrs.visibility {
        out.push("visibility");
    }
    if left.attrs.flags != right.attrs.flags {
        out.push("flags");
    }
    if left.attrs.signature != right.attrs.signature {
        out.push("signature");
    }
    if left.attrs.body_hash != right.attrs.body_hash {
        out.push("body_hash");
    }
    if left.attrs.signature_hash != right.attrs.signature_hash {
        out.push("signature_hash");
    }
    if left.attrs.parent != right.attrs.parent {
        out.push("parent");
    }
    if left.attrs.extra != right.attrs.extra {
        out.push("extra");
    }
    out
}

fn compare_edges(
    a: &dyn GraphQuery,
    b: &dyn GraphQuery,
    options: &CompareOptions,
    report: &mut GraphDiffReport,
) {
    let left = collect_edges(a);
    let right = collect_edges(b);
    let mut left_ix = 0usize;
    let mut right_ix = 0usize;
    while left_ix < left.len() && right_ix < right.len() {
        let l = left[left_ix].0;
        let r = right[right_ix].0;
        match l.cmp(&r) {
            std::cmp::Ordering::Less => {
                report.edges_only_a.push(l);
                left_ix += 1;
            }
            std::cmp::Ordering::Greater => {
                report.edges_only_b.push(r);
                right_ix += 1;
            }
            std::cmp::Ordering::Equal => {
                let fields = edge_field_diff(&left[left_ix].1, &right[right_ix].1, options);
                if !fields.is_empty() {
                    report.edges_differ.push((l, fields));
                }
                left_ix += 1;
                right_ix += 1;
            }
        }
    }
    report
        .edges_only_a
        .extend(left[left_ix..].iter().map(|(identity, _)| *identity));
    report
        .edges_only_b
        .extend(right[right_ix..].iter().map(|(identity, _)| *identity));
}

/// Every edge of a graph, keyed by identity and sorted. `O(E)` plus the `O(V log V)` sort.
fn collect_edges(graph: &dyn GraphQuery) -> Vec<(EdgeIdentity, EdgePayload)> {
    let mut out: Vec<(EdgeIdentity, EdgePayload)> = Vec::with_capacity(graph.edge_count());
    graph.for_each_node(&mut |node| {
        let mut edges: Vec<(EdgeIdentity, EdgePayload)> = Vec::new();
        graph.for_each_edge(node.key, Direction::Out, &EdgeFilter::ALL, &mut |edge| {
            edges.push((
                EdgeIdentity {
                    source: edge.source,
                    kind: edge.kind,
                    target: edge.target,
                },
                EdgePayload::of(edge),
            ));
            ControlFlow::Continue(())
        });
        out.extend(edges);
    });
    out.sort_by_key(|a| a.0);
    out
}

/// The comparable fields of one edge, copied out so the two sides can be diffed by value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EdgePayload {
    confidence: crate::Confidence,
    resolved_by: crate::ResolvedBy,
    provenance: crate::Provenance,
    flags: crate::EdgeFlags,
    location: Option<(String, u32, u32)>,
    occurrences: u32,
    origin_file: Option<String>,
}

impl EdgePayload {
    fn of(edge: EdgeRef<'_>) -> Self {
        Self {
            confidence: edge.confidence,
            resolved_by: edge.resolved_by,
            provenance: edge.provenance,
            flags: edge.flags,
            location: edge
                .location
                .map(|(path, line, col)| (path.to_owned(), line, col)),
            occurrences: edge.occurrences,
            origin_file: edge.origin_file.map(str::to_owned),
        }
    }
}

fn edge_field_diff(
    left: &EdgePayload,
    right: &EdgePayload,
    options: &CompareOptions,
) -> Vec<&'static str> {
    let mut out = Vec::new();
    if left.confidence != right.confidence {
        out.push("confidence");
    }
    if left.resolved_by != right.resolved_by {
        out.push("resolved_by");
    }
    if left.provenance != right.provenance {
        out.push("provenance");
    }
    if left.flags != right.flags {
        out.push("flags");
    }
    if !options.ignore_locations && left.location != right.location {
        out.push("location");
    }
    if left.occurrences != right.occurrences {
        out.push("occurrences");
    }
    if !options.ignore_locations && left.origin_file != right.origin_file {
        out.push("origin_file");
    }
    out
}

/// Every unresolved reference, keyed by `(file, ordinal)`.
fn compare_unresolved(a: &dyn GraphQuery, b: &dyn GraphQuery, report: &mut GraphDiffReport) {
    let left = collect_unresolved(a);
    let right = collect_unresolved(b);
    let mut left_keys: Vec<&(String, u32)> = left.keys().collect();
    let mut right_keys: Vec<&(String, u32)> = right.keys().collect();
    left_keys.sort();
    right_keys.sort();
    let mut left_ix = 0usize;
    let mut right_ix = 0usize;
    while left_ix < left_keys.len() && right_ix < right_keys.len() {
        match left_keys[left_ix].cmp(right_keys[right_ix]) {
            std::cmp::Ordering::Less => {
                report.unresolved_only_a.push(left_keys[left_ix].clone());
                left_ix += 1;
            }
            std::cmp::Ordering::Greater => {
                report.unresolved_only_b.push(right_keys[right_ix].clone());
                right_ix += 1;
            }
            std::cmp::Ordering::Equal => {
                let l = &left[left_keys[left_ix]];
                let r = &right[right_keys[right_ix]];
                let mut fields = Vec::new();
                if l.name != r.name {
                    fields.push("name");
                }
                if l.kind != r.kind {
                    fields.push("kind");
                }
                if l.reason != r.reason {
                    fields.push("reason");
                }
                if l.candidate_count != r.candidate_count {
                    fields.push("candidate_count");
                }
                if l.import_specifier != r.import_specifier {
                    fields.push("import_specifier");
                }
                if !fields.is_empty() {
                    report.unresolved_differ.push((
                        left_keys[left_ix].0.clone(),
                        left_keys[left_ix].1,
                        fields,
                    ));
                }
                left_ix += 1;
                right_ix += 1;
            }
        }
    }
    report
        .unresolved_only_a
        .extend(left_keys[left_ix..].iter().map(|key| (*key).clone()));
    report
        .unresolved_only_b
        .extend(right_keys[right_ix..].iter().map(|key| (*key).clone()));
}

type UnresolvedKey = (String, u32);

/// Every unresolved reference of a graph, keyed by `(file, ordinal)`.
fn collect_unresolved(
    graph: &dyn GraphQuery,
) -> std::collections::BTreeMap<UnresolvedKey, RefFields> {
    let mut out = std::collections::BTreeMap::new();
    graph.for_each_unresolved(&mut |reference| {
        out.insert(
            (reference.file.as_str().to_owned(), reference.ordinal),
            RefFields {
                name: reference.name.clone(),
                kind: reference.kind,
                reason: reference.reason,
                candidate_count: reference.candidate_count,
                import_specifier: reference.import_specifier.clone(),
            },
        );
    });
    out
}

/// The comparable fields of one unresolved reference.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RefFields {
    name: String,
    kind: analysis_ir::reference::RefKind,
    reason: crate::UnresolvedReason,
    candidate_count: u16,
    import_specifier: Option<String>,
}
