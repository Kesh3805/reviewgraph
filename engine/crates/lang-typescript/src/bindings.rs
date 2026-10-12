//! Module-level binding table: local name -> imported binding (TSA-004).

use std::collections::BTreeMap;

use analysis_ir::reference::BindingRef;

/// Maps module-level local names to the import binding that introduced them. TSA-005 uses it
/// to set `IrReference.import_binding`.
#[derive(Debug, Default, Clone)]
pub struct BindingTable {
    map: BTreeMap<String, BindingRef>,
}

impl BindingTable {
    /// Registers `local`; the first registration wins (a later duplicate is a TS error).
    pub fn insert(&mut self, local: &str, binding: BindingRef) {
        self.map.entry(local.to_owned()).or_insert(binding);
    }

    pub fn get(&self, local: &str) -> Option<BindingRef> {
        self.map.get(local).copied()
    }

    pub fn contains(&self, local: &str) -> bool {
        self.map.contains_key(local)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
