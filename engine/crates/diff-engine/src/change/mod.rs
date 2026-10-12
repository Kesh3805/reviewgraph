//! Change model (CHG-001..): what changed in each touched symbol, on both sides.

pub mod symbols;

pub use symbols::{
    build_changed_symbols, ChangeCfg, ChangedSymbolSet, SymbolFlags, SymbolRecord, SymbolVersion,
};
