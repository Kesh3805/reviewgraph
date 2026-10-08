//! Per-file symbol tables the linker resolves against (CG-005).
//!
//! One [`SymbolTable`] holds one [`FileSymbols`] per analyzed file. It is built once per
//! snapshot in `O(S)` over all symbols and is the *only* repository-wide state `link_file`
//! consults besides [`crate::linker::NameLookup`], which is what makes a single-file re-link
//! (INC-005) and an inbound re-link (INC-006) able to reuse the exact same resolution the full
//! build would have produced.
//!
//! Every map is a [`BTreeMap`], never a `HashMap`: the linker never iterates a hash map to
//! produce output, so resolution results cannot depend on hashing order.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use analysis_ir::module::{ImportKind, Imported};
use analysis_ir::symbol::{IrSymbol, LocalId, Modifiers, Visibility};
use analysis_ir::unit::ParsedUnit;
use review_core::location::{RepoPath, SourceRange};
use review_core::symbol::{Hash128, SymbolKind};

use crate::graph::{NodeFlags, NodeInput, NodeInputAttrs};
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;

/// A type written in source, e.g. `AuthService`, `users.Service` or `Map<string, User>`.
///
/// Stored as written so the same text produces the same lookup; [`TypeRef::base_name`] strips
/// the parts that cannot appear in an identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeRef(String);

impl TypeRef {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The identifier a name lookup can use: generic arguments, array and promise suffixes,
    /// a trailing `?` and a `readonly`/`keyof` prefix all come off, and a qualified name keeps
    /// only its last segment (an import resolves `users.Service` to the binding `Service`).
    #[must_use]
    pub fn base_name(&self) -> &str {
        let mut text = self.0.as_str();
        for prefix in ["readonly ", "keyof ", "typeof ", "Awaited<"] {
            if let Some(rest) = text.strip_prefix(prefix) {
                text = rest;
            }
        }
        let text = text.split('<').next().unwrap_or(text).trim();
        let text = text.trim_end_matches(['?', ']', ')', ' ']);
        text.rsplit('.').next().unwrap_or(text).trim()
    }

    /// The member part of a qualified type (`users.Service` → `Service`), which is the name an
    /// import binding would carry.
    #[must_use]
    pub fn member_name(&self) -> &str {
        let text = self.0.as_str();
        let head = text.split('<').next().unwrap_or(text).trim();
        head.rsplit('.').next().unwrap_or(head).trim()
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where an exported name comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    /// A symbol declared in this file.
    Local(NodeKey),
    /// `export { x } from "./y"` — another specifier that has to be followed.
    ReExport { specifier: String, name: String },
    /// `export * from "./y"` — every name of another module is visible through this one.
    StarFrom(String),
}

/// One import binding, flattened to what resolution needs: the specifier to resolve and the
/// name to look up on the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBindingInfo {
    pub local: String,
    /// `Named(x)` keeps `x`; `Default` and `CjsModule` look up `default`.
    pub imported: String,
    /// `import * as ns from "x"`: the binding is a namespace, so members resolve against the
    /// target module's export table under their own names.
    pub namespace: bool,
    pub type_only: bool,
    pub specifier: String,
    pub kind: ImportKind,
}

/// One symbol of one file, with everything the linker needs to emit edges about it.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInfo {
    pub key: NodeKey,
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub parent: Option<NodeKey>,
    /// Declared with an `export`/`pub` modifier.
    pub exported: bool,
    /// `abstract`, `static`, `async`, generated — the node flags this symbol carries.
    pub flags: NodeFlags,
    pub visibility: Visibility,
    pub signature: Option<String>,
    pub signature_hash: Option<Hash128>,
    pub body_hash: Option<Hash128>,
    pub range: SourceRange,
}

/// Everything the linker knows about one file's symbols.
#[derive(Debug, Clone)]
pub struct FileSymbols {
    pub path: RepoPath,
    /// Key of the `file:{path}` structural node, which is also `locals[0]`: the module symbol
    /// and the file are one node, not two.
    pub file_key: NodeKey,
    /// Local id → key. Index 0 is the module symbol and maps to [`Self::file_key`].
    pub locals: Vec<NodeKey>,
    /// Local id → full symbol record. Index 0 is a placeholder for the module symbol.
    pub symbols: Vec<SymbolInfo>,
    /// Exported name → where it comes from.
    pub exports: BTreeMap<String, ExportTarget>,
    /// `export * from` specifiers, sorted so the star order is deterministic.
    pub star_reexports: BTreeSet<String>,
    /// Local binding name → the import it was bound by.
    pub imports: BTreeMap<String, ImportBindingInfo>,
    /// Class key → member name → member key.
    pub members: BTreeMap<NodeKey, BTreeMap<String, NodeKey>>,
    /// Class key → constructor parameter (property) name → its declared type.
    pub ctor_params: BTreeMap<NodeKey, BTreeMap<String, TypeRef>>,
    /// Class key → `extends`/`implements` type references, in source order.
    pub supers: BTreeMap<NodeKey, Vec<TypeRef>>,
    /// Any symbol with a declared type (`const x: T`, parameter `p: T`).
    pub declared_types: BTreeMap<NodeKey, TypeRef>,
}

impl FileSymbols {
    /// Symbol record for a local id, if the file has one.
    pub fn symbol(&self, local: LocalId) -> Option<&SymbolInfo> {
        self.symbols.get(local.0 as usize)
    }

    /// The node inputs for this file: the file node itself plus one node per declared symbol.
    ///
    /// The module symbol (`local_id == 0`) is represented by the file node, so it is skipped.
    pub fn node_inputs(&self) -> Vec<NodeInput> {
        let mut out = Vec::with_capacity(self.symbols.len());
        out.push(
            NodeInput::new(
                NodeId::file(&self.path),
                NodeKind::File,
                file_name(&self.path),
            )
            .qualified_name(self.path.as_str())
            .in_file(self.path.clone()),
        );
        for symbol in self.symbols.iter().skip(1) {
            out.push(
                NodeInput::new(symbol.id.clone(), symbol.kind, symbol.name.clone())
                    .qualified_name(symbol.qualified_name.clone())
                    .in_file(self.path.clone())
                    .with_range(symbol.range)
                    .with_attrs(NodeInputAttrs {
                        visibility: symbol.visibility,
                        flags: symbol.flags,
                        signature: symbol.signature.clone(),
                        body_hash: symbol.body_hash,
                        signature_hash: symbol.signature_hash,
                        parent: symbol.parent,
                        extra: Vec::new(),
                    }),
            );
        }
        out
    }
}

fn file_name(path: &RepoPath) -> String {
    path.as_str()
        .rsplit('/')
        .next()
        .unwrap_or(path.as_str())
        .to_owned()
}

/// Every file's symbols, keyed by path so the table iterates deterministically.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    files: BTreeMap<RepoPath, FileSymbols>,
    /// Snapshot-wide member table: class key → member name → member key.
    ///
    /// A member lookup must span files: `this.verify()` on a field typed `AuthService` looks the
    /// member up on a class declared in another module (cascade step 3).
    members: BTreeMap<NodeKey, BTreeMap<String, NodeKey>>,
    /// Snapshot-wide class-name index, for resolving a written type that no import binds.
    class_names: BTreeMap<String, Vec<NodeKey>>,
}

impl SymbolTable {
    /// Builds the table for a whole snapshot in `O(S)` over all symbols.
    pub fn build(units: &[Arc<ParsedUnit>]) -> Self {
        let mut table = Self::default();
        for unit in units {
            table.insert_file(unit);
        }
        table
    }

    /// Adds or replaces one file. Used by the incremental lane to rebuild a single file's
    /// entry (INC-005).
    pub fn insert(&mut self, file: FileSymbols) {
        self.reindex(&file);
        self.files.insert(file.path.clone(), file);
    }

    /// Removes a file and its contribution to the snapshot-wide indexes.
    pub fn remove(&mut self, path: &RepoPath) {
        let Some(file) = self.files.remove(path) else {
            return;
        };
        for symbol in file.symbols.iter().skip(1) {
            if let Some(parent) = symbol.parent {
                if let Some(members) = self.members.get_mut(&parent) {
                    members.remove(&symbol.name);
                    if members.is_empty() {
                        self.members.remove(&parent);
                    }
                }
            }
            if matches!(
                symbol.kind,
                NodeKind::Class | NodeKind::Interface | NodeKind::Struct | NodeKind::Enum
            ) {
                if let Some(keys) = self.class_names.get_mut(&symbol.name) {
                    keys.retain(|key| *key != symbol.key);
                    if keys.is_empty() {
                        self.class_names.remove(&symbol.name);
                    }
                }
            }
        }
    }

    fn insert_file(&mut self, unit: &ParsedUnit) {
        let file = file_symbols(unit);
        self.reindex(&file);
        self.files.insert(file.path.clone(), file);
    }

    /// Adds one file's contribution to the snapshot-wide indexes.
    fn reindex(&mut self, file: &FileSymbols) {
        for (class, members) in &file.members {
            self.members
                .entry(*class)
                .or_default()
                .extend(members.iter().map(|(name, key)| (name.clone(), *key)));
        }
        for symbol in file.symbols.iter().skip(1) {
            if matches!(
                symbol.kind,
                NodeKind::Class | NodeKind::Interface | NodeKind::Struct | NodeKind::Enum
            ) {
                self.class_names
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(symbol.key);
            }
        }
    }

    pub fn file(&self, path: &RepoPath) -> Option<&FileSymbols> {
        self.files.get(path)
    }

    pub fn get(&self, path: &RepoPath) -> Option<&FileSymbols> {
        self.files.get(path)
    }

    /// Files in path order.
    pub fn paths(&self) -> impl Iterator<Item = &RepoPath> {
        self.files.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&RepoPath, &FileSymbols)> {
        self.files.iter()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Every declared symbol in the snapshot, in `(path, local id)` order.
    pub fn symbols(&self) -> impl Iterator<Item = (&RepoPath, &SymbolInfo)> {
        self.files.iter().flat_map(|(path, file)| {
            file.symbols
                .iter()
                .enumerate()
                .filter(move |(raw, _)| *raw != 0)
                .map(move |(_, symbol)| (path, symbol))
        })
    }

    /// Node kind of a key, when the key belongs to a declared symbol of this table.
    pub fn kind_of(&self, key: &NodeKey) -> Option<NodeKind> {
        self.files
            .values()
            .find_map(|file| file.symbols.iter().find(|s| &s.key == key).map(|s| s.kind))
    }

    /// The members of `owner`, whatever file declared it.
    pub fn members_of(&self, owner: &NodeKey) -> Option<&BTreeMap<String, NodeKey>> {
        self.members.get(owner)
    }

    /// The `extends`/`implements` types of `owner`, whatever file declared it.
    pub fn supers_of(&self, owner: &NodeKey) -> Option<&Vec<TypeRef>> {
        self.files.values().find_map(|file| file.supers.get(owner))
    }

    /// Class-shaped symbols named `name`, sorted by key.
    pub fn classes_named(&self, name: &str) -> &[NodeKey] {
        self.class_names.get(name).map_or(&[], Vec::as_slice)
    }

    /// The file that declares `key`, if any.
    pub fn file_of_symbol(&self, key: &NodeKey) -> Option<&RepoPath> {
        self.files
            .values()
            .find(|file| file.symbols.iter().any(|s| &s.key == key))
            .map(|file| &file.path)
    }
}

/// The canonical `NodeId` of one IR symbol (ADR-005):
/// `{lang}:{module_path}#{qualified_name}/{kind}[~{ordinal}]`.
///
/// The module symbol (`symbols[0]`) has no id of its own — it is the `file:{path}` node — so
/// this returns `None` for it and the caller stores [`FileSymbols::file_key`] instead.
#[must_use]
pub fn symbol_node_id(unit: &ParsedUnit, symbol: &IrSymbol) -> Option<NodeId> {
    if symbol.local_id == LocalId(0) {
        return None;
    }
    let mut id = String::new();
    id.push_str(unit.language.id_prefix());
    id.push(':');
    id.push_str(unit.module_path.as_str());
    id.push('#');
    id.push_str(&symbol.qualified_name.join("."));
    id.push('/');
    id.push_str(symbol.kind.as_id_str());
    if symbol.ordinal > 0 {
        id.push('~');
        id.push_str(&symbol.ordinal.to_string());
    }
    Some(NodeId::from_canonical(id))
}

/// Reads one [`ParsedUnit`] into a [`FileSymbols`] record.
#[must_use]
pub fn file_symbols(unit: &ParsedUnit) -> FileSymbols {
    let file_key = NodeId::file(&unit.file).key();
    let mut locals: Vec<NodeKey> = Vec::with_capacity(unit.symbols.len());
    let mut symbols: Vec<SymbolInfo> = Vec::with_capacity(unit.symbols.len());
    let mut members: BTreeMap<NodeKey, BTreeMap<String, NodeKey>> = BTreeMap::new();
    let mut ctor_params: BTreeMap<NodeKey, BTreeMap<String, TypeRef>> = BTreeMap::new();
    let mut supers: BTreeMap<NodeKey, Vec<TypeRef>> = BTreeMap::new();
    let mut declared_types: BTreeMap<NodeKey, TypeRef> = BTreeMap::new();
    let mut key_of: BTreeMap<u32, NodeKey> = BTreeMap::new();
    let mut kind_of_local: BTreeMap<u32, NodeKind> = BTreeMap::new();

    for symbol in &unit.symbols {
        let key = symbol_node_id(unit, symbol)
            .map(|id| id.key())
            .unwrap_or(file_key);
        key_of.insert(symbol.local_id.0, key);
        kind_of_local.insert(
            symbol.local_id.0,
            NodeKind::from_ir_symbol_kind(symbol.kind),
        );
        let qualified = symbol.qualified_name.join(".");
        symbols.push(SymbolInfo {
            key,
            id: symbol_node_id(unit, symbol).unwrap_or_else(|| NodeId::file(&unit.file)),
            kind: NodeKind::from_ir_symbol_kind(symbol.kind),
            name: symbol.name.clone(),
            qualified_name: qualified,
            parent: symbol.parent.map(|parent| {
                key_of
                    .get(&parent.0)
                    .copied()
                    .unwrap_or_else(|| NodeId::file(&unit.file).key())
            }),
            exported: symbol.modifiers.contains(Modifiers::EXPORTED),
            flags: node_flags(symbol),
            visibility: symbol.visibility,
            signature: symbol.signature.clone(),
            signature_hash: Some(symbol.signature_hash),
            body_hash: Some(symbol.body_hash),
            range: symbol.range,
        });
        locals.push(key);
    }
    // `symbols` is index-aligned with `unit.symbols`, so a member's parent key was only
    // available when the parent came first in source order. Fill the rest in now that every
    // local id has a key.
    for (raw, symbol) in unit.symbols.iter().enumerate() {
        if let Some(parent) = symbol.parent {
            if let Some(slot) = symbols.get_mut(raw) {
                slot.parent = key_of.get(&parent.0).copied();
            }
        }
    }

    for symbol in &unit.symbols {
        let key = locals
            .get(symbol.local_id.0 as usize)
            .copied()
            .unwrap_or(file_key);
        let kind = kind_of_local
            .get(&symbol.local_id.0)
            .copied()
            .unwrap_or(NodeKind::Function);
        if let Some(parent) = key_of.get(&symbol.parent.map_or(0, |p| p.0)) {
            if symbol.parent.is_some() {
                members
                    .entry(*parent)
                    .or_default()
                    .insert(symbol.name.clone(), key);
            }
        }
        if kind == NodeKind::Constructor || kind == NodeKind::Method {
            if let Some(class) = enclosing_class(unit, symbol.local_id) {
                for param in &symbol.params {
                    let Some(type_text) = &param.type_text else {
                        continue;
                    };
                    ctor_params
                        .entry(class)
                        .or_default()
                        .insert(param.name.clone(), TypeRef::new(type_text.clone()));
                }
            }
        }
        if kind == NodeKind::Class || kind == NodeKind::Interface {
            let mut her = Vec::new();
            for base in &symbol.heritage.extends {
                her.push(TypeRef::new(base.clone()));
            }
            for base in &symbol.heritage.implements {
                her.push(TypeRef::new(base.clone()));
            }
            if !her.is_empty() {
                supers.insert(key, her);
            }
        }
        if let Some(declared) = &symbol.declared_type {
            declared_types.insert(key, TypeRef::new(declared.clone()));
        }
    }

    let mut exports: BTreeMap<String, ExportTarget> = BTreeMap::new();
    let mut star_reexports: BTreeSet<String> = BTreeSet::new();
    for export in &unit.exports {
        match export {
            analysis_ir::module::IrExport::Local {
                symbol,
                exported_as,
                ..
            } => {
                if let Some(key) = locals.get(symbol.0 as usize) {
                    exports.insert(exported_as.clone(), ExportTarget::Local(*key));
                }
            }
            analysis_ir::module::IrExport::Reexport {
                specifier,
                imported,
                exported_as,
                ..
            } => {
                let name = match imported {
                    Imported::Named(n) => n.clone(),
                    Imported::Default => "default".to_owned(),
                    Imported::Namespace | Imported::CjsModule => "default".to_owned(),
                };
                exports.insert(
                    exported_as.clone(),
                    ExportTarget::ReExport {
                        specifier: specifier.clone(),
                        name,
                    },
                );
            }
            analysis_ir::module::IrExport::StarReexport {
                specifier,
                as_namespace,
                ..
            } => {
                if let Some(alias) = as_namespace {
                    exports.insert(alias.clone(), ExportTarget::StarFrom(specifier.clone()));
                } else {
                    star_reexports.insert(specifier.clone());
                }
            }
            analysis_ir::module::IrExport::CjsExportsProperty { name, symbol } => {
                if let Some(local) = symbol.and_then(|s| locals.get(s.0 as usize)) {
                    exports.insert(name.clone(), ExportTarget::Local(*local));
                }
            }
            analysis_ir::module::IrExport::ExportAssignment { symbol } => {
                if let Some(local) = symbol.and_then(|s| locals.get(s.0 as usize)) {
                    exports.insert("default".to_owned(), ExportTarget::Local(*local));
                } else {
                    exports.insert("default".to_owned(), ExportTarget::Local(file_key));
                }
            }
            analysis_ir::module::IrExport::DefaultExpr { .. }
            | analysis_ir::module::IrExport::CjsModuleExports { .. } => {}
        }
    }

    let mut imports: BTreeMap<String, ImportBindingInfo> = BTreeMap::new();
    for import in &unit.imports {
        for binding in &import.bindings {
            let (imported, namespace) = match &binding.imported {
                Imported::Named(name) => (name.clone(), false),
                Imported::Default => ("default".to_owned(), false),
                Imported::Namespace => (String::new(), true),
                Imported::CjsModule => (String::new(), true),
            };
            imports.insert(
                binding.local.clone(),
                ImportBindingInfo {
                    local: binding.local.clone(),
                    imported,
                    namespace,
                    type_only: binding.type_only || import.type_only,
                    specifier: import.specifier.clone(),
                    kind: import.kind,
                },
            );
        }
    }

    FileSymbols {
        path: unit.file.clone(),
        file_key,
        locals,
        symbols,
        exports,
        star_reexports,
        imports,
        members,
        ctor_params,
        supers,
        declared_types,
    }
}

/// The nearest enclosing class or interface of `local`, if any.
fn enclosing_class(unit: &ParsedUnit, local: LocalId) -> Option<NodeKey> {
    let by_id: BTreeMap<u32, &IrSymbol> = unit.symbols.iter().map(|s| (s.local_id.0, s)).collect();
    let mut cursor = by_id.get(&local.0)?.parent;
    while let Some(parent) = cursor {
        let symbol = by_id.get(&parent.0)?;
        let kind = NodeKind::from_ir_symbol_kind(symbol.kind);
        if matches!(kind, NodeKind::Class | NodeKind::Interface) {
            return symbol_node_id(unit, symbol).map(|id| id.key());
        }
        cursor = symbol.parent;
    }
    None
}

/// Node flags derived from the IR modifier set and syntactic kind (CG-004's `NodeFlags`).
fn node_flags(symbol: &IrSymbol) -> NodeFlags {
    let mut flags = NodeFlags::EMPTY;
    if symbol.modifiers.contains(Modifiers::EXPORTED) {
        flags |= NodeFlags::EXPORTED;
    }
    if symbol.modifiers.contains(Modifiers::ABSTRACT) {
        flags |= NodeFlags::ABSTRACT;
    }
    if symbol.modifiers.contains(Modifiers::STATIC) {
        flags |= NodeFlags::STATIC;
    }
    if symbol.modifiers.contains(Modifiers::ASYNC) {
        flags |= NodeFlags::ASYNC;
    }
    if symbol.kind == SymbolKind::EnumMember {
        flags |= NodeFlags::ENUM_MEMBER;
    }
    flags
}
