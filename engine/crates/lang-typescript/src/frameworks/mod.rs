//! Framework adapters (ADR-006). They run inside the analyzer after declarations, read the
//! decorators already on the symbols and the file's import bindings, and emit neutral
//! `IrFrameworkFact`s. NestJS knowledge lives only here (target-architecture section 3.1).

pub mod bullmq;
pub mod config;
pub mod jest;
pub mod nestjs;
pub mod typeorm;
pub mod util;

use analysis_ir::module::Imported;
use analysis_ir::{
    AnalyzerConfig, AttrValue, ConstValue, IrDecorator, IrExpr, IrFrameworkFact, IrImport,
    IrSymbol, LocalId,
};
use review_core::location::{Position, SourceRange};
use tree_sitter::Node;

use crate::bindings::BindingTable;
use crate::diagnostics::DiagnosticSink;
use crate::text::Source;

/// What a local name was imported as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub specifier: String,
    /// The exported name, `default` for a default import, `*` for a namespace import.
    pub imported: String,
}

/// Per-file inputs produced by the earlier visitors.
#[derive(Debug, Clone, Copy)]
pub struct FileInfo<'a> {
    pub path: &'a str,
    pub symbols: &'a [IrSymbol],
    pub imports: &'a [IrImport],
    pub bindings: &'a BindingTable,
}

/// Shared state for one file's adapters.
#[derive(Debug)]
pub struct FrameworkCtx<'a> {
    pub root: Node<'a>,
    pub src: &'a Source<'a>,
    pub path: &'a str,
    pub bindings: &'a BindingTable,
    pub imports: &'a [IrImport],
    pub symbols: &'a [IrSymbol],
    pub sink: &'a mut DiagnosticSink,
    pub facts: Vec<IrFrameworkFact>,
}

/// Confidence when the decorator is bound to the framework package.
pub const CONF_BOUND: f32 = 0.95;
/// Confidence for an unbound identifier with the framework's name (barrel re-export).
pub const CONF_UNBOUND: f32 = 0.7;

impl FrameworkCtx<'_> {
    /// The import behind a top-level local name.
    pub fn binding(&self, local: &str) -> Option<Resolved> {
        let reference = self.bindings.get(local)?;
        let import = self.imports.get(reference.import as usize)?;
        let binding = import.bindings.get(reference.binding as usize)?;
        let imported = match &binding.imported {
            Imported::Named(name) => name.clone(),
            Imported::Default => "default".to_owned(),
            Imported::Namespace | Imported::CjsModule => "*".to_owned(),
        };
        Some(Resolved {
            specifier: import.specifier.clone(),
            imported,
        })
    }

    /// Whether any import specifier starts with `prefix`.
    pub fn imports_from_scope(&self, prefix: &str) -> bool {
        self.imports.iter().any(|i| i.specifier.starts_with(prefix))
    }

    /// Whether the file imports exactly `specifier`.
    pub fn imports_from(&self, specifier: &str) -> bool {
        self.imports.iter().any(|i| i.specifier == specifier)
    }

    /// Confidence that `decorator` is the `wanted` export of `package`: bound import (also
    /// aliased or through a namespace import) 0.95, an unbound identifier of that name 0.7,
    /// anything else `None`.
    pub fn decorator_origin(
        &self,
        decorator: &IrDecorator,
        package: &str,
        wanted: &str,
    ) -> Option<f32> {
        self.name_origin(&decorator.name, &[package], wanted)
    }

    /// Like [`Self::decorator_origin`] for a callee or decorator name that may come from any of
    /// `packages`.
    pub fn name_origin(&self, name: &str, packages: &[&str], wanted: &str) -> Option<f32> {
        let mut parts = name.split('.');
        let root = parts.next()?;
        let rest: Vec<&str> = parts.collect();
        match (self.binding(root), rest.as_slice()) {
            (Some(b), []) if packages.contains(&b.specifier.as_str()) && b.imported == wanted => {
                Some(CONF_BOUND)
            }
            (Some(b), [last])
                if packages.contains(&b.specifier.as_str())
                    && b.imported == "*"
                    && *last == wanted =>
            {
                Some(CONF_BOUND)
            }
            (None, []) if root == wanted => Some(CONF_UNBOUND),
            _ => None,
        }
    }

    /// Like [`Self::name_origin`] for any `@nestjs/*` package.
    pub fn nest_origin(&self, name: &str, wanted: &str) -> Option<f32> {
        let mut parts = name.split('.');
        let root = parts.next()?;
        let rest: Vec<&str> = parts.collect();
        match (self.binding(root), rest.as_slice()) {
            (Some(b), []) if b.specifier.starts_with("@nestjs/") && b.imported == wanted => {
                Some(CONF_BOUND)
            }
            (Some(b), [last])
                if b.specifier.starts_with("@nestjs/") && b.imported == "*" && *last == wanted =>
            {
                Some(CONF_BOUND)
            }
            (None, []) if root == wanted => Some(CONF_UNBOUND),
            _ => None,
        }
    }

    /// The innermost symbol whose range contains `at`; the module symbol when none does.
    pub fn owner(&self, at: Position) -> LocalId {
        let mut best: Option<&IrSymbol> = None;
        for symbol in self.symbols {
            if !(symbol.range.start <= at && at <= symbol.range.end) {
                continue;
            }
            best = match best {
                None => Some(symbol),
                Some(current) => {
                    let inner = symbol.range.start > current.range.start
                        || (symbol.range.start == current.range.start
                            && symbol.range.end < current.range.end);
                    if inner {
                        Some(symbol)
                    } else {
                        Some(current)
                    }
                }
            };
        }
        best.map_or(LocalId(0), |s| s.local_id)
    }

    pub fn symbol(&self, id: LocalId) -> Option<&IrSymbol> {
        self.symbols.get(id.0 as usize)
    }

    /// The class that contains `id` (the symbol itself when it is a class).
    pub fn class_of(&self, id: LocalId) -> Option<&IrSymbol> {
        let mut current = self.symbol(id)?;
        for _ in 0..32 {
            if current.kind == review_core::symbol::SymbolKind::Class {
                return Some(current);
            }
            current = self.symbol(current.parent?)?;
        }
        None
    }

    /// `A.b` form of a symbol's qualified name, the string the graph mapper looks symbols up by.
    pub fn qname(&self, id: LocalId) -> String {
        self.symbol(id)
            .map(|s| s.qualified_name.join("."))
            .unwrap_or_default()
    }

    /// A string value: a literal, a no-substitution template, or a same-file constant or enum
    /// member (`QueueNames.MAIL`).
    pub fn const_str(&self, expr: &IrExpr) -> Option<String> {
        let parts: Vec<String> = match expr {
            IrExpr::Str(s) => return Some(s.clone()),
            IrExpr::Template {
                raw,
                has_subst: false,
            } => return Some(raw.clone()),
            IrExpr::Ident(n) => vec![n.clone()],
            IrExpr::Member(parts) => parts.clone(),
            _ => return None,
        };
        self.symbols
            .iter()
            .find(|s| s.qualified_name == parts && s.const_value.is_some())
            .and_then(|s| match &s.const_value {
                Some(ConstValue::Str(v)) => Some(v.clone()),
                _ => None,
            })
    }

    /// Adds a fact.
    pub fn emit(
        &mut self,
        adapter: &str,
        kind: analysis_ir::FrameworkFactKind,
        symbol: Option<LocalId>,
        attrs: std::collections::BTreeMap<String, AttrValue>,
        range: SourceRange,
        confidence: f32,
    ) {
        self.facts.push(IrFrameworkFact {
            adapter: adapter.to_owned(),
            kind,
            symbol,
            attrs,
            range,
            confidence,
        });
    }
}

/// Runs every enabled and detected adapter over one parsed file.
pub fn run(
    root: Node<'_>,
    src: &Source<'_>,
    file: FileInfo<'_>,
    cfg: &AnalyzerConfig,
    sink: &mut DiagnosticSink,
) -> Vec<IrFrameworkFact> {
    let enabled = |id: &str| match &cfg.enabled_adapters {
        None => true,
        Some(set) => set.contains(id),
    };
    let mut ctx = FrameworkCtx {
        root,
        src,
        path: file.path,
        bindings: file.bindings,
        imports: file.imports,
        symbols: file.symbols,
        sink,
        facts: Vec::new(),
    };
    if enabled(nestjs::ADAPTER) && nestjs::detect(&ctx, cfg) {
        nestjs::run(&mut ctx);
    }
    if enabled(typeorm::ADAPTER) && typeorm::detect(&ctx, cfg) {
        typeorm::run(&mut ctx);
    }
    if enabled(bullmq::ADAPTER) && bullmq::detect(&ctx, cfg) {
        bullmq::run(&mut ctx);
    }
    if enabled(jest::ADAPTER) && jest::detect(&ctx) {
        jest::run(&mut ctx);
    }
    if enabled(config::ADAPTER) && config::detect(&ctx) {
        config::run(&mut ctx);
    }
    ctx.facts
}
