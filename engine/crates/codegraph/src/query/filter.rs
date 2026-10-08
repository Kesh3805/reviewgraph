//! Edge filters: a kind set plus a confidence floor (CG-007).
//!
//! Every traversal takes an [`EdgeFilter`] rather than a bare kind list, so "only `CALLS` I am
//! confident in" is one value instead of two arguments that callers can disagree about.

use crate::graph::EdgeData;
use crate::{Confidence, EdgeKindSet, EdgeSelector};

/// What an edge must look like to be followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeFilter {
    /// The stored kinds that pass.
    pub kinds: EdgeKindSet,
    /// Inclusive floor; edges below it are skipped.
    pub min_confidence: Confidence,
}

impl EdgeFilter {
    /// Every kind at any confidence.
    pub const ALL: Self = Self {
        kinds: EdgeKindSet::ALL,
        min_confidence: Confidence::MIN,
    };

    /// Nothing: an empty kind set matches no edge whatever the confidence.
    pub const NONE: Self = Self {
        kinds: EdgeKindSet::EMPTY,
        min_confidence: Confidence::MIN,
    };

    pub const fn new(kinds: EdgeKindSet, min_confidence: Confidence) -> Self {
        Self {
            kinds,
            min_confidence,
        }
    }

    /// The kind set, at any confidence.
    pub const fn kinds(kinds: EdgeKindSet) -> Self {
        Self::new(kinds, Confidence::MIN)
    }

    /// Does `edge` pass?
    pub fn matches(&self, edge: &EdgeData) -> bool {
        self.kinds.contains(edge.kind) && edge.confidence >= self.min_confidence
    }

    /// Splits a wire-level selector list into the two filters a traversal needs: the out
    /// filter built from stored kinds, and the in filter built from the reverse views
    /// (`CALLED_BY` and friends are answered from the reverse index, so they belong on the
    /// `In` side).
    ///
    /// Asking for in-edges of a *stored* kind is not this function's job — pass the kinds
    /// directly to [`crate::GraphQuery::for_each_edge`] with [`crate::Direction::In`].
    pub fn from_selectors(selectors: &[EdgeSelector]) -> (Self, Self) {
        let mut out = EdgeKindSet::EMPTY;
        let mut inn = EdgeKindSet::EMPTY;
        for selector in selectors {
            let (kind, direction) = selector.underlying();
            match direction {
                crate::Direction::Out => out.insert(kind),
                crate::Direction::In => inn.insert(kind),
                crate::Direction::Both => {
                    out.insert(kind);
                    inn.insert(kind);
                }
            }
        }
        (Self::kinds(out), Self::kinds(inn))
    }
}

impl Default for EdgeFilter {
    fn default() -> Self {
        Self::ALL
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::{EdgeKind, NodeIx, Provenance, ResolvedBy, ReverseView};

    fn edge(kind: EdgeKind, confidence: Confidence) -> EdgeData {
        EdgeData {
            source: NodeIx::new(0),
            target: NodeIx::new(1),
            kind,
            resolved_by: ResolvedBy::NameUnique,
            provenance: Provenance::Linker,
            flags: crate::EdgeFlags::from_bits(0),
            confidence,
            origin_file: None,
            line: 0,
            col: 0,
            occurrences: 1,
        }
    }

    #[test]
    fn matches_requires_both_kind_and_confidence() {
        let edge = edge(EdgeKind::Calls, Confidence::from_f32(0.6));
        assert!(EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Calls)).matches(&edge));
        assert!(!EdgeFilter::new(EdgeKindSet::of(EdgeKind::Calls), Confidence::MAX).matches(&edge));
        assert!(!EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Reads)).matches(&edge));
        assert!(EdgeFilter::ALL.matches(&edge));
        assert!(!EdgeFilter::NONE.matches(&edge));
    }

    #[test]
    fn selectors_split_stored_kinds_from_reverse_views() {
        let selectors = [
            EdgeSelector::from(EdgeKind::Calls),
            EdgeSelector::from(ReverseView::CalledBy),
        ];
        let (out, inn) = EdgeFilter::from_selectors(&selectors);
        assert_eq!(out.kinds, EdgeKindSet::of(EdgeKind::Calls));
        assert!(inn.kinds.contains(EdgeKind::Calls));
        assert!(!out.kinds.contains(EdgeKind::DependsOn));
        assert!(out.min_confidence.is_min() && inn.min_confidence.is_min());
    }

    #[test]
    fn default_filter_is_everything() {
        assert_eq!(EdgeFilter::default(), EdgeFilter::ALL);
    }
}
