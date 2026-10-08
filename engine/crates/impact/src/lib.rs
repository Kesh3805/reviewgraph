//! `impact` crate: the impact graph, change clustering and the risk engine
//! (target-architecture §2, §3.7).
//!
//! The crate is pure: it reads graphs through [`codegraph::GraphQuery`] and the change model
//! through the narrow input types of [`input`], performs no I/O and calls no model.

pub mod error;
pub mod graph;
pub mod input;
mod metrics;
pub mod risk;

pub use error::{Error, Result};
pub use graph::{
    build_impact, ImpactBudget, ImpactElement, ImpactGraph, ImpactInputs, Relation, SymbolImpact,
};
pub use input::ChangeSet;
