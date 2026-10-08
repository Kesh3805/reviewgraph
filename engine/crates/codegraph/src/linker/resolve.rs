//! The resolution cascade (CG-005).
//!
//! Every `IrReference` walks the same steps, first success wins, and the step that succeeded is
//! recorded as the edge's [`ResolvedBy`], which fixes its confidence through
//! [`crate::confidence::confidence_of`]:
//!
//! 1. [`ResolvedBy::Import`] — the name is bound by an import; the resolver turns the specifier
//!    into a file (or a package), and `ReExport`/`StarFrom` chains are followed with cycle
//!    detection up to `max_reexport_depth`.
//! 2. [`ResolvedBy::ThisMember`] — `this.x()`: the enclosing class's member table, then its
//!    superclasses, nearest first.
//! 3. [`ResolvedBy::DiConstructor`] — the receiver is a constructor parameter property with a
//!    declared type; that type's members, then its supers.
//! 4. [`ResolvedBy::TypeAnnotation`] — the receiver is a local or parameter with a declared type.
//! 5. [`ResolvedBy::NameUnique`] — exactly one name-index candidate of a compatible kind class.
//! 6. [`ResolvedBy::NameAmbiguous`] — two to `max_ambiguous_fanout` candidates: one edge each,
//!    flagged `DYNAMIC`. More than that is [`UnresolvedReason::Ambiguous`] with the count.
//! 7. [`UnresolvedReason::NotFound`].
//!
//! Nothing here iterates a `HashMap`: candidates come from `BTreeMap` lookups and from
//! already-sorted slices, so `link_all` and any single-file `link_file` agree.

use std::collections::BTreeSet;
use std::fmt;

use analysis_ir::module::ImportKind;
use analysis_ir::reference::{IrReference, ReceiverHint, RefKind};
use analysis_ir::symbol::AttrValue;
use analysis_ir::traits::{
    ModuleResolver, Resolution, ResolveKind, UnresolvedReason as ResolverReason,
};
use analysis_ir::unit::ParsedUnit;
use review_core::location::RepoPath;

use crate::confidence;
use crate::edge::{Edge, EdgeFlags, Location, Provenance, ResolvedBy};
use crate::edge_kind::EdgeKind;
use crate::graph::{UnresolvedReason, UnresolvedRef};
use crate::linker::name_index::NameLookup;
use crate::linker::symbol_table::{
    ExportTarget, FileSymbols, ImportBindingInfo, SymbolTable, TypeRef,
};
use crate::linker::LinkConfig;
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;

/// What resolving the references of one file produced.
#[derive(Debug, Default)]
pub struct ResolveOutput {
    /// Edges the references produced, in reference order.
    pub edges: Vec<Edge>,
    /// One entry per specifier that resolved to a package outside the repository.
    pub package_edges: Vec<Edge>,
    /// References that produced no edge, with the reason why.
    pub unresolved: Vec<UnresolvedRef>,
    /// Names handed to the name index.
    pub consulted_names: BTreeSet<String>,
    /// Files whose export table was consulted.
    pub consulted_files: BTreeSet<RepoPath>,
    /// Synthetic package nodes discovered while following imports: `(ecosystem, name)`.
    pub packages: BTreeSet<(String, String)>,
}

/// Resolution state for one file, threaded through every reference it declares.
///
/// `Debug` prints the tables by size: the name index and the resolver are trait objects owned by
/// other crates.
pub struct ResolveContext<'a> {
    unit: &'a ParsedUnit,
    file: &'a FileSymbols,
    tables: &'a SymbolTable,
    names: &'a dyn NameLookup,
    resolver: &'a dyn ModuleResolver,
    config: &'a LinkConfig,
    edges: Vec<Edge>,
    package_edges: Vec<Edge>,
    unresolved: Vec<UnresolvedRef>,
    consulted_names: BTreeSet<String>,
    consulted_files: BTreeSet<RepoPath>,
    packages: BTreeSet<(String, String)>,
    /// Specifiers already turned into an `IMPORTS`/`DEPENDS_ON` pair, so N references through
    /// one import produce one edge pair rather than N.
    linked_specifiers: BTreeSet<String>,
}

impl fmt::Debug for ResolveContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolveContext")
            .field("file", &self.file.path.as_str())
            .field("edges", &self.edges.len())
            .field("unresolved", &self.unresolved.len())
            .finish_non_exhaustive()
    }
}

impl<'a> ResolveContext<'a> {
    pub fn new(
        unit: &'a ParsedUnit,
        file: &'a FileSymbols,
        tables: &'a SymbolTable,
        names: &'a dyn NameLookup,
        resolver: &'a dyn ModuleResolver,
        config: &'a LinkConfig,
    ) -> Self {
        Self {
            unit,
            file,
            tables,
            names,
            resolver,
            config,
            edges: Vec::new(),
            package_edges: Vec::new(),
            unresolved: Vec::new(),
            consulted_names: BTreeSet::new(),
            consulted_files: BTreeSet::new(),
            packages: BTreeSet::new(),
            linked_specifiers: BTreeSet::new(),
        }
    }

    /// Resolves every reference of the file, in IR order.
    pub fn run(mut self) -> ResolveOutput {
        for (ordinal, reference) in self.unit.references.iter().enumerate() {
            let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
            let Some(from) = self.file.locals.get(reference.from.0 as usize).copied() else {
                continue;
            };
            let location = Location::new(
                self.unit.file.clone(),
                reference.range.start.line,
                reference.range.start.column,
            );
            self.resolve(reference, from, &location, ordinal);
        }
        ResolveOutput {
            edges: self.edges,
            package_edges: self.package_edges,
            unresolved: self.unresolved,
            consulted_names: self.consulted_names,
            consulted_files: self.consulted_files,
            packages: self.packages,
        }
    }

    fn specifier_of(&self, reference: &IrReference) -> Option<String> {
        let binding = reference.import_binding?;
        self.unit
            .imports
            .get(binding.import as usize)
            .map(|import| import.specifier.clone())
    }

    /// One reference through the whole cascade.
    fn resolve(
        &mut self,
        reference: &IrReference,
        from: NodeKey,
        location: &Location,
        ordinal: u32,
    ) {
        // Step 1: the name comes from an import.
        if let Some(binding) = self.import_binding_of(reference) {
            match self.resolve_via_import(&binding, reference) {
                ImportOutcome::Keys(keys) => {
                    self.emit(
                        reference,
                        from,
                        &keys,
                        ResolvedBy::Import,
                        location,
                        EdgeFlags::EMPTY,
                    );
                    return;
                }
                ImportOutcome::Package => {
                    self.unresolved.push(self.unresolved_ref(
                        reference,
                        from,
                        location,
                        ordinal,
                        UnresolvedReason::External,
                        0,
                    ));
                    return;
                }
                ImportOutcome::Failed(reason) => {
                    self.unresolved
                        .push(self.unresolved_ref(reference, from, location, ordinal, reason, 0));
                    return;
                }
            }
        }

        // Steps 2-4: the receiver carries enough type information.
        if let Some(rule) = receiver_rule(&reference.receiver) {
            if let Some(owner) = self.owner_of(reference, rule) {
                if let Some(target) = self.lookup_member_through_supers(&owner, &reference.name, 0)
                {
                    self.emit(
                        reference,
                        from,
                        &[target],
                        rule.resolved_by(),
                        location,
                        EdgeFlags::EMPTY,
                    );
                    return;
                }
            }
        }

        // Steps 5-6: a bare name.
        self.resolve_by_name(reference, from, location, ordinal);
    }

    fn unresolved_ref(
        &self,
        reference: &IrReference,
        from: NodeKey,
        location: &Location,
        ordinal: u32,
        reason: UnresolvedReason,
        candidate_count: u16,
    ) -> UnresolvedRef {
        UnresolvedRef {
            file: self.unit.file.clone(),
            ordinal,
            from: Some(from),
            name: reference.name.clone(),
            kind: reference.kind,
            import_specifier: self.specifier_of(reference),
            location: location.clone(),
            reason,
            candidate_count,
        }
    }

    fn import_binding_of(&self, reference: &IrReference) -> Option<ImportBindingInfo> {
        let binding = reference.import_binding?;
        let import = self.unit.imports.get(binding.import as usize)?;
        let local = import.bindings.get(binding.binding as usize)?;
        self.file.imports.get(&local.local).cloned()
    }

    /// Step 1 in full: specifier → file or package, then the export chain.
    fn resolve_via_import(
        &mut self,
        binding: &ImportBindingInfo,
        reference: &IrReference,
    ) -> ImportOutcome {
        let kind = match binding.kind {
            ImportKind::CjsRequire => ResolveKind::Require,
            ImportKind::Dynamic => ResolveKind::Dynamic,
            ImportKind::Esm | ImportKind::SideEffect | ImportKind::ImportEquals => {
                ResolveKind::Import
            }
        };
        match self
            .resolver
            .resolve(&self.unit.file, &binding.specifier, kind)
        {
            Resolution::File { path, .. } => {
                self.consulted_files.insert(path.clone());
                let Some(target) = self.tables.get(&path) else {
                    // The resolver produced a path this snapshot does not have: refuse it
                    // rather than linking to a file the graph will not contain.
                    return ImportOutcome::Failed(UnresolvedReason::NotFound);
                };
                let wanted = if binding.namespace {
                    reference.name.clone()
                } else if binding.imported.is_empty() {
                    "default".to_owned()
                } else {
                    binding.imported.clone()
                };
                self.lookup_export(target, &wanted, &mut BTreeSet::new(), 0)
            }
            Resolution::External {
                ecosystem, name, ..
            } => {
                self.link_package(&ecosystem, &name, &binding.specifier);
                ImportOutcome::Package
            }
            Resolution::Builtin { name } => {
                self.link_package("node", &name, &binding.specifier);
                ImportOutcome::Package
            }
            Resolution::Unresolved { reason } => ImportOutcome::Failed(match reason {
                ResolverReason::NotFound | ResolverReason::OutsideRepository => {
                    UnresolvedReason::NotFound
                }
                ResolverReason::Ambiguous => UnresolvedReason::Ambiguous,
                ResolverReason::NonLiteral | ResolverReason::Unsupported => {
                    UnresolvedReason::ResolverError
                }
            }),
        }
    }

    /// Follows `exports[name]`, then any `ReExport`/`StarFrom` chain it points at.
    ///
    /// `visited` breaks cycles and `depth` enforces `max_reexport_depth`; both failures become
    /// unresolved references rather than errors, so a cyclic barrel never turns into a hang.
    fn lookup_export(
        &mut self,
        file: &'a FileSymbols,
        name: &str,
        visited: &mut BTreeSet<NodeKey>,
        depth: u8,
    ) -> ImportOutcome {
        if !visited.insert(file.file_key) {
            return ImportOutcome::Failed(UnresolvedReason::ReexportCycle);
        }
        if depth >= self.config.max_reexport_depth {
            return ImportOutcome::Failed(UnresolvedReason::DepthExceeded);
        }
        if let Some(target) = file.exports.get(name) {
            return match target {
                ExportTarget::Local(key) => ImportOutcome::Keys(vec![*key]),
                ExportTarget::ReExport { specifier, name } => {
                    self.re_export(file, specifier, name, visited, depth)
                }
                ExportTarget::StarFrom(specifier) => {
                    self.re_export(file, specifier, name, visited, depth)
                }
            };
        }
        // Not an explicit export: it may still arrive through `export *`.
        let stars: Vec<String> = file.star_reexports.iter().cloned().collect();
        let mut cycle = false;
        for specifier in stars {
            self.consulted_files.insert(file.path.clone());
            let Some(next) = self.resolve_specifier_to_file(&file.path, &specifier) else {
                continue;
            };
            if visited.contains(&next.file_key) {
                cycle = true;
                break;
            }
            match self.lookup_export(next, name, visited, depth + 1) {
                ImportOutcome::Keys(keys) => return ImportOutcome::Keys(keys),
                ImportOutcome::Failed(UnresolvedReason::ReexportCycle) => {
                    cycle = true;
                    break;
                }
                ImportOutcome::Failed(_) | ImportOutcome::Package => {}
            }
        }
        if cycle {
            ImportOutcome::Failed(UnresolvedReason::ReexportCycle)
        } else {
            ImportOutcome::Failed(UnresolvedReason::NotFound)
        }
    }

    fn re_export(
        &mut self,
        from: &'a FileSymbols,
        specifier: &str,
        name: &str,
        visited: &mut BTreeSet<NodeKey>,
        depth: u8,
    ) -> ImportOutcome {
        let Some(next) = self.resolve_specifier_to_file(&from.path, specifier) else {
            return ImportOutcome::Failed(UnresolvedReason::ResolverError);
        };
        self.lookup_export(next, name, visited, depth + 1)
    }

    /// Specifier → target file, or `None` when the resolver points outside the snapshot.
    fn resolve_specifier_to_file(
        &self,
        from: &RepoPath,
        specifier: &str,
    ) -> Option<&'a FileSymbols> {
        let Resolution::File { path, .. } =
            self.resolver.resolve(from, specifier, ResolveKind::Import)
        else {
            return None;
        };
        // Reborrow through the `'a` table reference rather than `&self`, so the returned
        // `FileSymbols` does not keep `self` borrowed while the chain is followed mutably.
        let tables: &'a SymbolTable = self.tables;
        tables.get(&path)
    }

    /// Records the package node an external specifier maps to and emits its two file-level
    /// edges, once per specifier however many references share it.
    ///
    /// The ecosystem comes from the resolver's answer, so `Cargo` and `npm` dependencies get
    /// their own package nodes without the linker hard-coding one.
    fn link_package(&mut self, ecosystem: &str, name: &str, specifier: &str) {
        self.packages
            .insert((ecosystem.to_owned(), name.to_owned()));
        let Ok(package) = NodeId::package(ecosystem, name) else {
            return;
        };
        let Ok(path) = RepoPath::new(self.unit.file.as_str()) else {
            return;
        };
        if !self.linked_specifiers.insert(specifier.to_owned()) {
            return;
        }
        let location = Location::new(path, 0, 0);
        for kind in [EdgeKind::Imports, EdgeKind::DependsOn] {
            self.package_edges.push(
                Edge::new(
                    kind,
                    self.file.file_key,
                    package.key(),
                    confidence::confidence_of(ResolvedBy::Import),
                    ResolvedBy::Import,
                    Provenance::Linker,
                )
                .with_location(location.clone()),
            );
        }
    }

    /// Steps 5-6: the name index.
    fn resolve_by_name(
        &mut self,
        reference: &IrReference,
        from: NodeKey,
        location: &Location,
        ordinal: u32,
    ) {
        let wants = compatible_kinds(reference.kind);
        let table = if is_member_access(&reference.receiver) {
            self.names.members(&reference.name)
        } else {
            self.names.top(&reference.name)
        };
        self.consulted_names.insert(reference.name.clone());
        let candidates: Vec<NodeKey> = table
            .iter()
            .copied()
            .filter(|key| {
                self.tables
                    .kind_of(key)
                    .is_none_or(|kind| wants.is_empty() || wants.contains(&kind))
            })
            .collect();
        match candidates.len() {
            0 => self.unresolved.push(self.unresolved_ref(
                reference,
                from,
                location,
                ordinal,
                UnresolvedReason::NotFound,
                0,
            )),
            1 => self.emit(
                reference,
                from,
                &candidates,
                ResolvedBy::NameUnique,
                location,
                EdgeFlags::EMPTY,
            ),
            n if n <= usize::from(self.config.max_ambiguous_fanout) => self.emit(
                reference,
                from,
                &candidates,
                ResolvedBy::NameAmbiguous,
                location,
                EdgeFlags::DYNAMIC,
            ),
            n => self.unresolved.push(self.unresolved_ref(
                reference,
                from,
                location,
                ordinal,
                UnresolvedReason::Ambiguous,
                u16::try_from(n).unwrap_or(u16::MAX),
            )),
        }
    }

    /// The type whose members a receiver step should look in.
    fn owner_of(&self, reference: &IrReference, rule: Cascade) -> Option<NodeKey> {
        match (&reference.receiver, rule) {
            (ReceiverHint::This | ReceiverHint::Super, Cascade::ThisMember) => {
                let info = self.file.symbol(reference.from)?;
                info.parent
                    .or_else(|| self.file.locals.get(reference.from.0 as usize).copied())
            }
            (
                ReceiverHint::ThisField {
                    field,
                    declared_type,
                },
                Cascade::DiConstructor,
            ) => {
                let type_ref = declared_type.clone().or_else(|| {
                    let owner = self.file.symbol(reference.from)?.parent?;
                    self.file
                        .ctor_params
                        .get(&owner)
                        .and_then(|params| params.get(field))
                        .map(TypeRef::to_string)
                })?;
                self.resolve_type_ref(&type_ref)
            }
            (
                ReceiverHint::Identifier {
                    declared_type,
                    name,
                    ..
                },
                Cascade::TypeAnnotation,
            ) => {
                let type_ref = declared_type.clone().or_else(|| {
                    self.file
                        .locals
                        .get(reference.from.0 as usize)
                        .and_then(|key| self.file.declared_types.get(key))
                        .map(TypeRef::to_string)
                });
                match type_ref {
                    Some(text) => self.resolve_type_ref(&text),
                    None => {
                        let _ = name;
                        None
                    }
                }
            }
            _ => None,
        }
    }

    /// Turns a written type into a key: an import binding first, then a file-local class, then a
    /// class of that name anywhere in the snapshot.
    fn resolve_type_ref(&self, type_ref: &str) -> Option<NodeKey> {
        let parsed = TypeRef::new(type_ref);
        let member = parsed.member_name();
        let base = parsed.base_name();
        if let Some(binding) = self.file.imports.get(member) {
            let name = if binding.namespace {
                member
            } else if binding.imported.is_empty() {
                "default"
            } else {
                binding.imported.as_str()
            };
            if let Some(target) =
                self.resolve_specifier_to_file(&self.file.path, &binding.specifier)
            {
                if let Some(ExportTarget::Local(key)) = target.exports.get(name) {
                    return Some(*key);
                }
            }
        }
        if let Some(found) = self
            .file
            .members
            .keys()
            .copied()
            .find(|key| self.symbol_name(key) == Some(base))
        {
            return Some(found);
        }
        let candidates = self.tables.classes_named(base);
        candidates.first().copied()
    }

    fn symbol_name(&self, key: &NodeKey) -> Option<&str> {
        self.file
            .symbols
            .iter()
            .find(|s| &s.key == key)
            .map(|s| s.name.as_str())
    }

    /// Member lookup on `owner`, then on its superclasses, nearest first.
    ///
    /// The member table spans the whole snapshot: a field typed `AuthService` puts `owner` in
    /// another module, so a file-local lookup would never find its members.
    fn lookup_member_through_supers(
        &mut self,
        owner: &NodeKey,
        name: &str,
        depth: u8,
    ) -> Option<NodeKey> {
        if depth >= self.config.max_super_depth {
            return None;
        }
        if let Some(found) = self.tables.members_of(owner).and_then(|m| m.get(name)) {
            return Some(*found);
        }
        let sups: Vec<TypeRef> = self.tables.supers_of(owner).cloned().unwrap_or_default();
        for sup in sups {
            let Some(sup_key) = self.resolve_type_ref(&sup.to_string()) else {
                continue;
            };
            if let Some(found) = self.lookup_member_through_supers(&sup_key, name, depth + 1) {
                return Some(found);
            }
        }
        None
    }

    /// Turns resolved targets into the edges the reference kind calls for.
    fn emit(
        &mut self,
        reference: &IrReference,
        from: NodeKey,
        targets: &[NodeKey],
        rule: ResolvedBy,
        location: &Location,
        extra_flags: EdgeFlags,
    ) {
        for target in targets {
            self.edges.extend(edges_for(
                reference,
                from,
                *target,
                rule,
                location,
                extra_flags,
            ));
        }
    }
}

/// Which cascade step a receiver belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cascade {
    ThisMember,
    DiConstructor,
    TypeAnnotation,
}

impl Cascade {
    const fn resolved_by(self) -> ResolvedBy {
        match self {
            Self::ThisMember => ResolvedBy::ThisMember,
            Self::DiConstructor => ResolvedBy::DiConstructor,
            Self::TypeAnnotation => ResolvedBy::TypeAnnotation,
        }
    }
}

/// What step 1 concluded.
enum ImportOutcome {
    Keys(Vec<NodeKey>),
    /// The specifier lives outside the repository; a `pkg:` node now exists for it.
    Package,
    Failed(UnresolvedReason),
}

fn receiver_rule(receiver: &ReceiverHint) -> Option<Cascade> {
    match receiver {
        ReceiverHint::This | ReceiverHint::Super => Some(Cascade::ThisMember),
        ReceiverHint::ThisField { .. } => Some(Cascade::DiConstructor),
        ReceiverHint::Identifier { .. } => Some(Cascade::TypeAnnotation),
        _ => None,
    }
}

/// Node kinds a reference may point at. Empty means "any kind".
fn compatible_kinds(kind: RefKind) -> &'static [NodeKind] {
    const CALLABLE: &[NodeKind] = &[
        NodeKind::Function,
        NodeKind::Method,
        NodeKind::Constructor,
        NodeKind::Handler,
        NodeKind::JobHandler,
        NodeKind::CliCommand,
    ];
    const TYPES: &[NodeKind] = &[
        NodeKind::Class,
        NodeKind::Interface,
        NodeKind::Struct,
        NodeKind::Trait,
        NodeKind::Enum,
        NodeKind::TypeAlias,
    ];
    match kind {
        RefKind::Call | RefKind::DiInjection | RefKind::FrameworkRef => CALLABLE,
        RefKind::New | RefKind::TypeRef | RefKind::Extends | RefKind::Implements => TYPES,
        RefKind::Decorator | RefKind::ValueRead | RefKind::JsxElement => &[],
    }
}

/// The edges one reference contributes, per the CG-005 kind mapping.
fn edges_for(
    reference: &IrReference,
    from: NodeKey,
    target: NodeKey,
    rule: ResolvedBy,
    location: &Location,
    extra_flags: EdgeFlags,
) -> Vec<Edge> {
    let confidence = confidence::confidence_of(rule);
    let make = |kind: EdgeKind, flags: EdgeFlags| {
        Edge::new(kind, from, target, confidence, rule, Provenance::Linker)
            .with_location(location.clone())
            .with_flags(flags.union(extra_flags))
    };
    match reference.kind {
        RefKind::Call => vec![make(EdgeKind::Calls, EdgeFlags::EMPTY)],
        RefKind::New => {
            vec![
                make(EdgeKind::Calls, EdgeFlags::INSTANTIATES),
                make(EdgeKind::UsesType, EdgeFlags::TYPE_ONLY),
            ]
        }
        RefKind::TypeRef => {
            if is_return_position(reference) {
                vec![make(EdgeKind::ReturnsType, EdgeFlags::TYPE_ONLY)]
            } else {
                vec![make(EdgeKind::UsesType, EdgeFlags::TYPE_ONLY)]
            }
        }
        RefKind::Extends => vec![make(EdgeKind::Extends, EdgeFlags::EMPTY)],
        RefKind::Implements => vec![make(EdgeKind::Implements, EdgeFlags::EMPTY)],
        RefKind::Decorator => vec![make(EdgeKind::References, EdgeFlags::DECORATOR)],
        RefKind::DiInjection => vec![make(EdgeKind::UsesType, EdgeFlags::TYPE_ONLY)],
        RefKind::FrameworkRef | RefKind::ValueRead | RefKind::JsxElement => {
            vec![make(EdgeKind::References, EdgeFlags::EMPTY)]
        }
    }
}

/// True when the analyzer recorded this type reference as a return position
/// (`attrs["position"] == "return"`), which turns `USES_TYPE` into `RETURNS_TYPE`.
fn is_return_position(reference: &IrReference) -> bool {
    matches!(reference.attrs.get("position"), Some(AttrValue::Str(value)) if value == "return")
}

/// True when the reference is a member access (`a.b`, `this.b`) rather than a bare name, which
/// decides whether the member or the top-level name index is consulted.
fn is_member_access(receiver: &ReceiverHint) -> bool {
    matches!(
        receiver,
        ReceiverHint::This
            | ReceiverHint::Super
            | ReceiverHint::ThisField { .. }
            | ReceiverHint::Identifier { .. }
            | ReceiverHint::Chain { .. }
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use review_core::location::{Position, SourceRange};

    fn reference(kind: RefKind) -> IrReference {
        let range =
            SourceRange::new(Position::new(3, 2).unwrap(), Position::new(3, 9).unwrap()).unwrap();
        IrReference {
            from: analysis_ir::symbol::LocalId(1),
            kind,
            name: "target".to_owned(),
            receiver: ReceiverHint::None,
            import_binding: None,
            range,
            arg_count: 0,
            in_test_block: false,
            attrs: std::collections::BTreeMap::new(),
        }
    }

    fn location() -> Location {
        let path = RepoPath::new("src/a.ts").unwrap();
        Location::new(path, 3, 2)
    }

    fn ids(edges: &[Edge]) -> Vec<(EdgeKind, NodeKey, NodeKey)> {
        edges.iter().map(|e| (e.kind, e.source, e.target)).collect()
    }

    #[test]
    fn kind_mapping_follows_the_cg_005_table() {
        let from = NodeId::from_canonical("ts:src/a.ts#A/f/function").key();
        let to = NodeId::from_canonical("ts:src/b.ts#B/g/function").key();

        let calls = edges_for(
            &reference(RefKind::Call),
            from,
            to,
            ResolvedBy::NameUnique,
            &location(),
            EdgeFlags::EMPTY,
        );
        assert_eq!(ids(&calls), vec![(EdgeKind::Calls, from, to)]);
        assert_eq!(
            calls[0].confidence,
            confidence::confidence_of(ResolvedBy::NameUnique)
        );

        let new = edges_for(
            &reference(RefKind::New),
            from,
            to,
            ResolvedBy::NameUnique,
            &location(),
            EdgeFlags::EMPTY,
        );
        assert_eq!(
            ids(&new),
            vec![(EdgeKind::Calls, from, to), (EdgeKind::UsesType, from, to)]
        );
        assert!(new[0].flags.contains(EdgeFlags::INSTANTIATES));
        assert!(new[1].flags.contains(EdgeFlags::TYPE_ONLY));

        let mut ret = reference(RefKind::TypeRef);
        ret.attrs
            .insert("position".to_owned(), AttrValue::Str("return".to_owned()));
        assert_eq!(
            ids(&edges_for(
                &ret,
                from,
                to,
                ResolvedBy::NameUnique,
                &location(),
                EdgeFlags::EMPTY
            )),
            vec![(EdgeKind::ReturnsType, from, to)]
        );

        let decorator = edges_for(
            &reference(RefKind::Decorator),
            from,
            to,
            ResolvedBy::NameUnique,
            &location(),
            EdgeFlags::EMPTY,
        );
        assert_eq!(ids(&decorator), vec![(EdgeKind::References, from, to)]);
        assert!(decorator[0].flags.contains(EdgeFlags::DECORATOR));

        let read = edges_for(
            &reference(RefKind::ValueRead),
            from,
            to,
            ResolvedBy::NameUnique,
            &location(),
            EdgeFlags::DYNAMIC,
        );
        assert!(read[0].flags.contains(EdgeFlags::DYNAMIC));
    }

    #[test]
    fn receiver_hints_select_the_cascade_step() {
        assert_eq!(
            receiver_rule(&ReceiverHint::This),
            Some(Cascade::ThisMember)
        );
        assert_eq!(
            receiver_rule(&ReceiverHint::Super),
            Some(Cascade::ThisMember)
        );
        assert_eq!(
            receiver_rule(&ReceiverHint::ThisField {
                field: "svc".to_owned(),
                declared_type: None
            }),
            Some(Cascade::DiConstructor)
        );
        assert_eq!(
            receiver_rule(&ReceiverHint::Identifier {
                name: "svc".to_owned(),
                declared_type: Some("S".to_owned())
            }),
            Some(Cascade::TypeAnnotation)
        );
        assert_eq!(receiver_rule(&ReceiverHint::None), None);
        assert_eq!(Cascade::ThisMember.resolved_by(), ResolvedBy::ThisMember);
        assert_eq!(
            Cascade::DiConstructor.resolved_by(),
            ResolvedBy::DiConstructor
        );
        assert_eq!(
            Cascade::TypeAnnotation.resolved_by(),
            ResolvedBy::TypeAnnotation
        );
    }

    #[test]
    fn compatible_kinds_separate_callables_from_types() {
        assert!(compatible_kinds(RefKind::Call).contains(&NodeKind::Function));
        assert!(!compatible_kinds(RefKind::Call).contains(&NodeKind::Class));
        assert!(compatible_kinds(RefKind::New).contains(&NodeKind::Class));
        assert!(compatible_kinds(RefKind::Extends).contains(&NodeKind::Interface));
        assert!(compatible_kinds(RefKind::Decorator).is_empty());
    }

    #[test]
    fn member_access_decides_which_index_is_consulted() {
        assert!(is_member_access(&ReceiverHint::This));
        assert!(is_member_access(&ReceiverHint::Identifier {
            name: "a".to_owned(),
            declared_type: None
        }));
        assert!(is_member_access(&ReceiverHint::Chain {
            root: Box::new(ReceiverHint::None),
            segments: vec!["a".to_owned()],
        }));
        assert!(!is_member_access(&ReceiverHint::None));
        assert!(!is_member_access(&ReceiverHint::Computed));
        assert!(!is_member_access(&ReceiverHint::CallResult {
            callee: "a".to_owned()
        }));
    }
}
