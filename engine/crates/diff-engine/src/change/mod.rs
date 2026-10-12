//! Change model (CHG-001..): what changed in each touched symbol, on both sides.

pub mod delta;
pub mod graphs;
pub mod symbols;
pub mod versions;

pub use delta::{DeltaEntry, DeltaKind, SymbolDelta};
pub use graphs::GraphPair;
pub use symbols::{build_changed_symbols, ChangeCfg, ChangedSymbolSet, SymbolFlags, SymbolRecord};
pub use versions::SymbolVersion;
