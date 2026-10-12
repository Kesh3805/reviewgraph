//! Change model (CHG-001..): what changed in each touched symbol, on both sides.

pub mod classes;
pub mod classify;
pub mod delta;
pub mod fact_diff;
pub mod graphs;
pub mod symbols;
pub mod versions;

pub use classify::{
    classify_all, default_classifiers, Certainty, ClassifiedChange, Classifier, ClassifyCx,
    FactsAccess, FactsView,
};
pub use delta::{DeltaEntry, DeltaKind, SymbolDelta};
pub use fact_diff::{diff_facts, FactDiff, FactRef};
pub use graphs::GraphPair;
pub use symbols::{build_changed_symbols, ChangeCfg, ChangedSymbolSet, SymbolFlags, SymbolRecord};
pub use versions::SymbolVersion;
