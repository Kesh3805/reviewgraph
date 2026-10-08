//! `impact` crate: the impact graph, change clustering and the risk engine
//! (target-architecture §2, §3.7).
//!
//! The crate is pure: it reads graphs through [`codegraph::GraphQuery`] and the change model
//! through narrow input types, performs no I/O and calls no model.

pub mod error;
pub mod graph;

pub use error::{Error, Result};
pub use graph::{ImpactBudget, ImpactElement, ImpactGraph, Relation, SymbolImpact};
