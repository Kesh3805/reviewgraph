//! Head symbol table construction and symbol classification (INC-003).

pub mod diff;
pub mod head_tables;

pub use diff::{
    change_inputs, diff_changed_files, rename_hints, FileChangeInput, NameEntry, SymbolChangeSet,
    SymbolCounts, SymbolRecord,
};
pub use head_tables::{
    BaseSymbolSource, BaseUnitSource, HeadSymbolTables, MapUnitSource, MemoSymbolSource,
};
