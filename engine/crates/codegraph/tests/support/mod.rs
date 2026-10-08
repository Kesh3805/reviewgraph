//! Shared fixture builders for the `codegraph` integration tests.
//!
//! The linker and the framework mapper both consume `analysis_ir` output, so their tests need to
//! construct `ParsedUnit`s by hand. That construction is verbose and easy to get subtly wrong
//! (a `LocalId` that does not line up with `symbols[i]` fails far away from its cause), so it
//! lives here once and every test file includes it with `mod support;`.
//!
//! Not compiled outside the test tree: this module is never part of the crate.

#![allow(dead_code)]
//! Fixture builders use `unwrap` throughout: every argument is a literal checked by the type
//! system, so a failure here is a typo in the test rather than a runtime condition to handle.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use analysis_ir::framework::{FrameworkFactKind, IrFrameworkFact};
use analysis_ir::module::{Imported, IrExport, IrImport};
use analysis_ir::reference::{BindingRef, IrReference, ReceiverHint, RefKind};
use analysis_ir::symbol::{AttrValue, IrSymbol, LocalId, Modifiers, Visibility};
use analysis_ir::traits::{ModuleResolver, Resolution, ResolutionMethod, ResolveKind};
use analysis_ir::unit::{AnalyzerId, ParseStatus, ParsedUnit, UnitStats};
use review_core::language::{Dialect, Language};
use review_core::location::{ContentHash, Position, RepoPath, SourceRange};
use review_core::symbol::{ModulePath, SymbolKind};
use review_core::version::AnalyzerVersion;

/// A 1-based line range.
pub fn range(start: u32, end: u32) -> SourceRange {
    SourceRange::new(
        Position::new(start, 0).unwrap(),
        Position::new(end, 0).unwrap(),
    )
    .unwrap()
}

/// A validated repository-relative path.
pub fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

/// A `ParsedUnit` under construction. Symbols are appended in source order, so a symbol's
/// `LocalId` is its index and `local(1)` is the first declaration after the module symbol.
pub struct UnitBuilder {
    unit: ParsedUnit,
    next_local: u32,
}

impl UnitBuilder {
    /// A TypeScript unit at `raw_path`.
    pub fn new(raw_path: &str) -> Self {
        let repo_path = path(raw_path);
        let mut symbols = vec![IrSymbol::new(
            LocalId(0),
            SymbolKind::Module,
            "__module__",
            vec!["__module__".to_owned()],
            range(1, 1),
        )];
        symbols[0].visibility = Visibility::Public;
        Self {
            unit: ParsedUnit {
                ir_schema: 1,
                file: repo_path.clone(),
                module_path: ModulePath::of(&repo_path),
                language: Language::Typescript,
                dialect: Some(Dialect::Ts),
                content_hash: ContentHash::of(repo_path.as_str().as_bytes()),
                analyzer: AnalyzerId {
                    name: "test".to_owned(),
                    version: AnalyzerVersion::new(0, 0, 1),
                },
                status: ParseStatus::Ok,
                symbols,
                references: Vec::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                framework: Vec::new(),
                facts: Vec::new(),
                diagnostics: Vec::new(),
                stats: UnitStats::default(),
            },
            next_local: 1,
        }
    }

    /// Appends a top-level symbol and returns its `LocalId`.
    pub fn symbol(&mut self, kind: SymbolKind, name: &str, line: u32) -> LocalId {
        self.child(kind, name, None, line)
    }

    /// Appends a symbol nested inside `parent`.
    pub fn child(
        &mut self,
        kind: SymbolKind,
        name: &str,
        parent: Option<LocalId>,
        line: u32,
    ) -> LocalId {
        let local = LocalId(self.next_local);
        self.next_local += 1;
        let qualified = match parent {
            Some(parent) => {
                let parent_name = self
                    .unit
                    .symbols
                    .get(parent.0 as usize)
                    .map(|symbol| symbol.name.clone())
                    .unwrap_or_default();
                vec![parent_name, name.to_owned()]
            }
            None => vec![name.to_owned()],
        };
        let mut symbol = IrSymbol::new(local, kind, name, qualified, range(line, line + 4));
        symbol.parent = parent;
        symbol.visibility = Visibility::Public;
        self.unit.symbols.push(symbol);
        local
    }

    /// Marks the symbol at `local` as exported and records `File EXPORTS` for it.
    pub fn exported(&mut self, local: LocalId) {
        let exported_as = self.unit.symbols[local.0 as usize].name.clone();
        let symbol = &mut self.unit.symbols[local.0 as usize];
        symbol.modifiers = symbol.modifiers.union(Modifiers::EXPORTED);
        self.unit.exports.push(IrExport::Local {
            symbol: local,
            exported_as,
            type_only: false,
        });
    }

    /// Adds a named import from `specifier` and returns `(import index, binding index)`.
    pub fn import_named(
        &mut self,
        specifier: &str,
        local: &str,
        imported: &str,
        line: u32,
    ) -> (u32, u32) {
        let index = u32::try_from(self.unit.imports.len()).unwrap_or(u32::MAX);
        self.unit.imports.push(IrImport {
            specifier: specifier.to_owned(),
            kind: analysis_ir::module::ImportKind::Esm,
            type_only: false,
            bindings: vec![analysis_ir::module::ImportBinding {
                local: local.to_owned(),
                imported: Imported::Named(imported.to_owned()),
                type_only: false,
            }],
            range: range(line, line),
        });
        (index, 0)
    }

    /// Adds a namespace import (`import * as ns from "…"`) and returns `(import index, binding index)`.
    pub fn import_namespace(&mut self, specifier: &str, local: &str, line: u32) -> (u32, u32) {
        let index = u32::try_from(self.unit.imports.len()).unwrap_or(u32::MAX);
        self.unit.imports.push(IrImport {
            specifier: specifier.to_owned(),
            kind: analysis_ir::module::ImportKind::Esm,
            type_only: false,
            bindings: vec![analysis_ir::module::ImportBinding {
                local: local.to_owned(),
                imported: Imported::Namespace,
                type_only: false,
            }],
            range: range(line, line),
        });
        (index, 0)
    }

    /// `export { name } from "specifier"`.
    pub fn reexport(&mut self, specifier: &str, imported: &str, exported_as: &str) {
        self.unit.exports.push(IrExport::Reexport {
            specifier: specifier.to_owned(),
            imported: Imported::Named(imported.to_owned()),
            exported_as: exported_as.to_owned(),
            type_only: false,
        });
    }

    /// `export * from "specifier"`.
    pub fn star_reexport(&mut self, specifier: &str) {
        self.unit.exports.push(IrExport::StarReexport {
            specifier: specifier.to_owned(),
            as_namespace: None,
            type_only: false,
        });
    }

    /// A reference with no import binding and the given receiver hint.
    pub fn reference(&mut self, from: LocalId, kind: RefKind, name: &str, line: u32) {
        self.reference_with(from, kind, name, ReceiverHint::None, line, None);
    }

    /// A reference with an explicit receiver hint and optional import binding.
    pub fn reference_with(
        &mut self,
        from: LocalId,
        kind: RefKind,
        name: &str,
        receiver: ReceiverHint,
        line: u32,
        import: Option<BindingRef>,
    ) {
        self.unit.references.push(IrReference {
            from,
            kind,
            name: name.to_owned(),
            receiver,
            import_binding: import,
            range: range(line, line),
            arg_count: 0,
            in_test_block: false,
            attrs: BTreeMap::new(),
        });
    }

    /// A `this.x()` reference: the cascade's `ThisMember` step.
    pub fn this_member(&mut self, from: LocalId, name: &str, line: u32) {
        self.reference_with(from, RefKind::Call, name, ReceiverHint::This, line, None);
    }

    /// A framework fact.
    pub fn fact(&mut self, kind: FrameworkFactKind, attrs: Vec<(&str, AttrValue)>, line: u32) {
        self.unit.framework.push(IrFrameworkFact {
            adapter: "test".to_owned(),
            kind,
            symbol: None,
            attrs: attrs
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
            range: range(line, line + 1),
            confidence: 0.9,
        });
    }

    pub fn build(self) -> Arc<ParsedUnit> {
        Arc::new(self.unit)
    }
}

/// A `ModuleResolver` backed by an explicit table, plus a count of how often each specifier was
/// resolved (so a test can prove the cascade asked only once per import).
#[derive(Debug, Default)]
pub struct TableResolver {
    files: BTreeMap<(String, String), String>,
    external: BTreeMap<String, (String, String)>,
    builtins: BTreeMap<String, String>,
    queries: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

impl TableResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// `from` + `specifier` resolves to a file in the repository.
    pub fn file(mut self, from: &str, specifier: &str, target: &str) -> Self {
        self.files
            .insert((from.to_owned(), specifier.to_owned()), target.to_owned());
        self
    }

    /// `from` + `specifier` resolves to an external package.
    pub fn external(mut self, from: &str, specifier: &str, ecosystem: &str, name: &str) -> Self {
        self.external.insert(
            format!("{from}\u{0}{specifier}"),
            (ecosystem.to_owned(), name.to_owned()),
        );
        self
    }

    /// `from` + `specifier` resolves to a language builtin.
    pub fn builtin(mut self, from: &str, specifier: &str, name: &str) -> Self {
        self.builtins
            .insert(format!("{from}\u{0}{specifier}"), name.to_owned());
        self
    }

    /// How many resolver calls have been made, and for which specifiers.
    pub fn queries(&self) -> Vec<(String, String)> {
        self.queries
            .lock()
            .map(|log| log.clone())
            .unwrap_or_default()
    }
}

impl ModuleResolver for TableResolver {
    fn resolve(&self, from: &RepoPath, specifier: &str, _kind: ResolveKind) -> Resolution {
        if let Ok(mut log) = self.queries.lock() {
            log.push((from.as_str().to_owned(), specifier.to_owned()));
        }
        let key = format!("{}\u{0}{specifier}", from.as_str());
        if let Some(target) = self
            .files
            .get(&(from.as_str().to_owned(), specifier.to_owned()))
        {
            return Resolution::File {
                path: path(target),
                method: ResolutionMethod::Relative,
                confidence: 1.0,
            };
        }
        if let Some((ecosystem, name)) = self.external.get(&key) {
            return Resolution::External {
                ecosystem: ecosystem.clone(),
                name: name.clone(),
                subpath: None,
                version_range: None,
            };
        }
        if let Some(name) = self.builtins.get(&key) {
            return Resolution::Builtin { name: name.clone() };
        }
        Resolution::Unresolved {
            reason: analysis_ir::traits::UnresolvedReason::NotFound,
        }
    }
}

/// The id of a TypeScript symbol, as the linker builds it.
pub fn symbol_id(raw_path: &str, qualified: &str, kind: &str) -> codegraph::NodeId {
    let repo_path = path(raw_path);
    codegraph::NodeId::from_canonical(format!(
        "ts:{}#{qualified}/{kind}",
        ModulePath::of(&repo_path).as_str()
    ))
}

/// Human-readable dump of a link output, for the golden snapshot.
pub fn dump(output: &codegraph::LinkOutput) -> String {
    let mut out = String::new();
    out.push_str("NODES\n");
    for node in &output.nodes {
        out.push_str(&format!(
            "  {} {} {} file={}\n",
            node.kind,
            node.id.as_str(),
            node.name,
            node.file.as_ref().map_or("-", |file| file.as_str())
        ));
    }
    out.push_str("EDGES\n");
    for edge in &output.edges {
        out.push_str(&format!(
            "  {} -[{}]-> {} conf={} by={} prov={} flags={}\n",
            edge.source,
            edge.kind,
            edge.target,
            edge.confidence,
            edge.resolved_by,
            edge.provenance,
            edge.flags
        ));
    }
    out.push_str("UNRESOLVED\n");
    for reference in &output.unresolved {
        out.push_str(&format!(
            "  {}#{} {} {:?} reason={} candidates={}\n",
            reference.file.as_str(),
            reference.ordinal,
            reference.name,
            reference.kind,
            reference.reason,
            reference.candidate_count
        ));
    }
    out
}

/// The kind of an edge in a dump-friendly form.
pub fn edge_kinds(output: &codegraph::LinkOutput) -> Vec<String> {
    output
        .edges
        .iter()
        .map(|edge| format!("{}->{}:{}", edge.source, edge.target, edge.kind))
        .collect()
}

/// A readable dump of one file's framework mapping, so a failed assertion shows the output.
pub fn dump_output(out: &codegraph::FrameworkOutput) -> String {
    let mut text = String::from("NODES\n");
    for node in &out.nodes {
        text.push_str(&format!(
            "  {} {} {} extra={:?}\n",
            node.kind,
            node.id.as_str(),
            node.name,
            node.attrs.extra
        ));
    }
    text.push_str("EDGES\n");
    for edge in &out.edges {
        text.push_str(&format!(
            "  {} -[{}]-> {} conf={} flags={}\n",
            edge.source, edge.kind, edge.target, edge.confidence, edge.flags
        ));
    }
    text.push_str("REFINEMENTS\n");
    for (key, kind) in &out.refinements {
        text.push_str(&format!("  {key} -> {kind}\n"));
    }
    text.push_str("ISSUES\n");
    for issue in &out.issues {
        text.push_str(&format!("  {issue}\n"));
    }
    text.push_str("GLOBALS\n");
    for global in &out.global_facts {
        text.push_str(&format!("  {global:?}\n"));
    }
    text
}
