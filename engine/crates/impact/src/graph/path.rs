//! Paths and the merge rule for elements reached more than once (IMP-001).
//!
//! When a node is reached under the same relation by several paths, the best path wins:
//! highest `min_confidence`, then shortest distance, then the lexicographically smallest
//! sequence of node keys along the path. The losers are counted in `alt_paths`. The rule is a
//! total order, so the winner never depends on discovery order — which is what lets seeds be
//! expanded in parallel and still serialize identically.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use codegraph::{Confidence, NodeKey, NodeKind};

use super::budget::ImpactBudget;
use super::model::{
    EndpointAttrs, GraphSide, ImpactElement, PathStep, Relation, ResourceAttrs, TestMapping,
};

/// A path from a seed, with the bookkeeping the merge rule needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trail {
    pub steps: Vec<PathStep>,
    /// The seed followed by every node the path visits, in order.
    pub nodes: Vec<NodeKey>,
    pub min_confidence: Confidence,
    pub distance: u8,
}

impl Trail {
    /// The empty path at a seed.
    pub fn seed(seed: NodeKey) -> Self {
        Self {
            steps: Vec::new(),
            nodes: vec![seed],
            min_confidence: Confidence::MAX,
            distance: 0,
        }
    }

    /// The node the path ends at.
    pub fn end(&self) -> Option<NodeKey> {
        self.nodes.last().copied()
    }

    /// This path plus one step arriving at `next`. `hop` is false for steps that do not add
    /// distance (interface dispatch).
    pub fn extend(&self, step: PathStep, next: NodeKey, hop: bool) -> Self {
        let min_confidence = self.min_confidence.min(step.confidence);
        let mut steps = self.steps.clone();
        steps.push(step);
        let mut nodes = self.nodes.clone();
        nodes.push(next);
        Self {
            steps,
            nodes,
            min_confidence,
            distance: if hop {
                self.distance.saturating_add(1)
            } else {
                self.distance
            },
        }
    }

    /// Does the path already visit `node` (cycle guard)?
    pub fn visits(&self, node: NodeKey) -> bool {
        self.nodes.contains(&node)
    }
}

/// `Less` when `a` is the better path.
pub fn compare_trails(a: &Trail, b: &Trail) -> Ordering {
    b.min_confidence
        .cmp(&a.min_confidence)
        .then_with(|| a.distance.cmp(&b.distance))
        .then_with(|| a.nodes.cmp(&b.nodes))
}

/// Per-element payload that travels with the winning path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extras {
    pub endpoint: Option<EndpointAttrs>,
    pub test: Option<TestMapping>,
    pub resource: Option<ResourceAttrs>,
    pub touches: Vec<String>,
    pub missing_on_head: bool,
}

/// An element offered to an [`ElementSet`].
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub node: NodeKey,
    pub node_id: String,
    pub kind: NodeKind,
    pub trail: Trail,
    pub extras: Extras,
}

#[derive(Debug, Clone)]
struct Entry {
    candidate: Candidate,
    alt_paths: u16,
}

/// What [`ElementSet::offer`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// A new element.
    Inserted,
    /// An existing element whose best path was replaced.
    Improved,
    /// An existing element kept its path; the offer counted as an alternative.
    Alternative,
}

/// The elements of one seed, keyed by `(relation, node)`, merged by the best-path rule.
#[derive(Debug, Clone, Default)]
pub struct ElementSet {
    entries: BTreeMap<(Relation, NodeKey), Entry>,
}

impl ElementSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, relation: Relation, node: NodeKey) -> bool {
        self.entries.contains_key(&(relation, node))
    }

    /// Elements currently held for `relation`.
    pub fn count(&self, relation: Relation) -> u32 {
        self.entries
            .keys()
            .filter(|(held, _)| *held == relation)
            .count() as u32
    }

    /// Elements across every relation.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The best trail currently held for `(relation, node)`.
    pub fn trail(&self, relation: Relation, node: NodeKey) -> Option<&Trail> {
        self.entries
            .get(&(relation, node))
            .map(|entry| &entry.candidate.trail)
    }

    /// Nodes held for `relation`, with their trails, in key order.
    pub fn of(&self, relation: Relation) -> Vec<(NodeKey, &Trail)> {
        self.entries
            .iter()
            .filter(|((held, _), _)| *held == relation)
            .map(|((_, node), entry)| (*node, &entry.candidate.trail))
            .collect()
    }

    /// Offers a candidate under `relation`. Callers check caps before offering a node that is
    /// not yet [`Self::contains`]ed; improving an existing element never needs budget.
    pub fn offer(&mut self, relation: Relation, candidate: Candidate) -> Offer {
        let key = (relation, candidate.node);
        match self.entries.get_mut(&key) {
            None => {
                self.entries.insert(
                    key,
                    Entry {
                        candidate,
                        alt_paths: 0,
                    },
                );
                Offer::Inserted
            }
            Some(entry) => {
                entry.alt_paths = entry.alt_paths.saturating_add(1);
                if compare_trails(&candidate.trail, &entry.candidate.trail) == Ordering::Less {
                    entry.candidate = candidate;
                    Offer::Improved
                } else {
                    Offer::Alternative
                }
            }
        }
    }

    /// Mutable access to an element's extras (for attributes computed after insertion).
    pub fn extras_mut(&mut self, relation: Relation, node: NodeKey) -> Option<&mut Extras> {
        self.entries
            .get_mut(&(relation, node))
            .map(|entry| &mut entry.candidate.extras)
    }

    /// Freezes the set into sorted elements; `weak` is decided against `budget`.
    pub fn into_elements(self, budget: &ImpactBudget) -> Vec<ImpactElement> {
        let mut elements: Vec<ImpactElement> = self
            .entries
            .into_iter()
            .map(|((relation, _), entry)| {
                let Candidate {
                    node,
                    node_id,
                    kind,
                    trail,
                    extras,
                } = entry.candidate;
                ImpactElement {
                    node,
                    node_id,
                    kind,
                    relation,
                    distance: trail.distance,
                    min_confidence: trail.min_confidence,
                    weak: trail.min_confidence < budget.min_confidence,
                    path: trail.steps,
                    alt_paths: entry.alt_paths,
                    endpoint: extras.endpoint,
                    test: extras.test,
                    resource: extras.resource,
                    touches: extras.touches,
                    missing_on_head: extras.missing_on_head,
                }
            })
            .collect();
        super::model::sort_elements(&mut elements);
        elements
    }
}

/// A path step read from a graph edge.
pub fn step(
    from: NodeKey,
    edge: codegraph::EdgeKind,
    to: NodeKey,
    confidence: Confidence,
    graph: GraphSide,
) -> PathStep {
    PathStep {
        from,
        edge,
        to,
        confidence,
        graph,
    }
}
