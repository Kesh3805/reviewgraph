//! [`GraphPair`]: the base and head graphs a change model may consult.

use std::fmt;

use codegraph::query::GraphQuery;
use review_core::location::DiffSide;

/// Read access to the graphs of both sides. Either side may be absent (local runs without a
/// graph store use units only); every consumer degrades to unit facts in that case.
#[derive(Clone, Copy, Default)]
pub struct GraphPair<'a> {
    /// The base graph.
    pub base: Option<&'a dyn GraphQuery>,
    /// The head graph (overlay or full rebuild).
    pub head: Option<&'a dyn GraphQuery>,
}

impl fmt::Debug for GraphPair<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphPair")
            .field("base", &self.base.is_some())
            .field("head", &self.head.is_some())
            .finish()
    }
}

impl<'a> GraphPair<'a> {
    /// No graphs: unit-only analysis.
    pub const fn none() -> Self {
        Self {
            base: None,
            head: None,
        }
    }

    /// Both graphs.
    pub fn new(base: &'a dyn GraphQuery, head: &'a dyn GraphQuery) -> Self {
        Self {
            base: Some(base),
            head: Some(head),
        }
    }

    /// The graph of `side`, if present.
    pub fn graph(&self, side: DiffSide) -> Option<&'a dyn GraphQuery> {
        match side {
            DiffSide::Base => self.base,
            DiffSide::Head => self.head,
        }
    }
}
