//! Framework adapters (ADR-006). They run inside the analyzer after declarations, read the
//! decorators already on the symbols and the file's import bindings, and emit neutral
//! `IrFrameworkFact`s. NestJS knowledge lives only here (target-architecture section 3.1).

pub mod nestjs;

use std::collections::BTreeMap;

use analysis_ir::{AnalyzerConfig, IrDecorator, IrFrameworkFact, IrSymbol};
use tree_sitter::Node;

use crate::diagnostics::DiagnosticSink;
use crate::text::Source;

/// What a local name was imported as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub specifier: String,
    /// The exported name, `default` for a default import, `*` for a namespace import.
    pub imported: String,
}

/// Top-level import bindings of one file, by local name.
#[derive(Debug, Default)]
pub struct BindingTable {
    pub by_local: BTreeMap<String, Binding>,
}

impl BindingTable {
    pub fn collect(root: Node<'_>, src: &Source<'_>) -> Self {
        let mut table = Self::default();
        let mut cursor = root.walk();
        for stmt in root.children(&mut cursor) {
            if stmt.kind() != "import_statement" {
                continue;
            }
            let Some(source) = stmt.child_by_field_name("source") else {
                continue;
            };
            let specifier = crate::visit::expr::string_content(source, src);
            let mut c1 = stmt.walk();
            for clause in stmt.children(&mut c1) {
                if clause.kind() != "import_clause" {
                    continue;
                }
                let mut c2 = clause.walk();
                for part in clause.children(&mut c2) {
                    match part.kind() {
                        "identifier" => table.add(&src.text(part), &specifier, "default"),
                        "namespace_import" => {
                            let mut c3 = part.walk();
                            for id in part.children(&mut c3) {
                                if id.kind() == "identifier" {
                                    table.add(&src.text(id), &specifier, "*");
                                }
                            }
                        }
                        "named_imports" => {
                            let mut c3 = part.walk();
                            for spec in part.children(&mut c3) {
                                if spec.kind() != "import_specifier" {
                                    continue;
                                }
                                if let Some(name) = spec.child_by_field_name("name") {
                                    let imported = src.text(name).into_owned();
                                    let local = spec.child_by_field_name("alias").map_or_else(
                                        || imported.clone(),
                                        |a| src.text(a).into_owned(),
                                    );
                                    table.add(&local, &specifier, &imported);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        table
    }

    fn add(&mut self, local: &str, specifier: &str, imported: &str) {
        self.by_local.insert(
            local.to_owned(),
            Binding {
                specifier: specifier.to_owned(),
                imported: imported.to_owned(),
            },
        );
    }

    pub fn get(&self, local: &str) -> Option<&Binding> {
        self.by_local.get(local)
    }

    pub fn imports_from_scope(&self, prefix: &str) -> bool {
        self.by_local
            .values()
            .any(|b| b.specifier.starts_with(prefix))
    }
}

/// Shared state for one file's adapters.
#[derive(Debug)]
pub struct FrameworkCtx<'a> {
    pub src: &'a Source<'a>,
    pub bindings: BindingTable,
    pub symbols: &'a [IrSymbol],
    pub sink: &'a mut DiagnosticSink,
    pub facts: Vec<IrFrameworkFact>,
}

/// Confidence when the decorator is bound to the framework package.
pub const CONF_BOUND: f32 = 0.95;
/// Confidence for an unbound identifier with the framework's name (barrel re-export).
pub const CONF_UNBOUND: f32 = 0.7;

impl FrameworkCtx<'_> {
    /// Confidence that `decorator` is the `wanted` export of `package`: bound import (also
    /// aliased or through a namespace import) 0.95, an unbound identifier of that name 0.7,
    /// anything else `None`.
    pub fn decorator_origin(
        &self,
        decorator: &IrDecorator,
        package: &str,
        wanted: &str,
    ) -> Option<f32> {
        let mut parts = decorator.name.split('.');
        let root = parts.next()?;
        let rest: Vec<&str> = parts.collect();
        match (self.bindings.get(root), rest.as_slice()) {
            (Some(b), []) if b.specifier == package && b.imported == wanted => Some(CONF_BOUND),
            (Some(b), [last]) if b.specifier == package && b.imported == "*" && *last == wanted => {
                Some(CONF_BOUND)
            }
            (None, []) if root == wanted => Some(CONF_UNBOUND),
            _ => None,
        }
    }
}

/// Runs every enabled and detected adapter over one parsed file.
pub fn run(
    root: Node<'_>,
    src: &Source<'_>,
    symbols: &[IrSymbol],
    cfg: &AnalyzerConfig,
    sink: &mut DiagnosticSink,
) -> Vec<IrFrameworkFact> {
    let enabled = |id: &str| match &cfg.enabled_adapters {
        None => true,
        Some(set) => set.contains(id),
    };
    let mut ctx = FrameworkCtx {
        src,
        bindings: BindingTable::collect(root, src),
        symbols,
        sink,
        facts: Vec::new(),
    };
    if enabled(nestjs::ADAPTER) && nestjs::detect(&ctx, cfg) {
        nestjs::run(&mut ctx);
    }
    ctx.facts
}
