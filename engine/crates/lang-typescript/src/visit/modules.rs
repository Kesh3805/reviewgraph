//! Import and export extraction, ESM and CommonJS (TSA-004).
//!
//! Specifiers are kept verbatim; resolving them to files is TSA-009.

use analysis_ir::reference::BindingRef;
use analysis_ir::symbol::Modifiers;
use analysis_ir::{
    DiagCode, DiagSeverity, ImportBinding, ImportKind, Imported, IrExport, IrExpr, IrImport,
    LocalId,
};
use tree_sitter::Node;

use crate::bindings::BindingTable;
use crate::diagnostics::DiagnosticSink;
use crate::kinds::{field, kind};
use crate::text::Source;
use crate::visit::declarations::Table;
use crate::visit::expr::{string_content, to_ir_expr};
use crate::visit::{depth_limit, MAX_VISIT_DEPTH};

/// Longest specifier kept; longer ones are dropped with a diagnostic.
const MAX_SPECIFIER_BYTES: usize = 1024;

/// Imports, exports and the module-level binding table of one file.
#[derive(Debug, Default)]
pub struct ModuleFacts {
    pub imports: Vec<IrImport>,
    pub exports: Vec<IrExport>,
    pub bindings: BindingTable,
}

struct Extractor<'a, 't> {
    src: &'a Source<'a>,
    table: &'a Table<'t>,
    sink: &'a mut DiagnosticSink,
    out: ModuleFacts,
}

/// Extracts every import and export below `root`.
pub fn collect<'a, 't>(
    root: Node<'t>,
    src: &'a Source<'a>,
    table: &'a Table<'t>,
    sink: &'a mut DiagnosticSink,
) -> ModuleFacts {
    let mut ex = Extractor {
        src,
        table,
        sink,
        out: ModuleFacts::default(),
    };
    let mut cursor = root.walk();
    for stmt in root.named_children(&mut cursor) {
        match stmt.kind() {
            kind::IMPORT_STATEMENT => ex.esm_import(stmt),
            kind::EXPORT_STATEMENT => ex.export_statement(stmt),
            kind::EXPRESSION_STATEMENT => ex.cjs_export_statement(stmt),
            _ => {}
        }
        ex.walk(stmt, 0, false, false);
    }
    ex.out
}

fn has_child_kind(node: Node<'_>, wanted: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == wanted);
    found
}

fn is_function_like(k: &str) -> bool {
    matches!(
        k,
        kind::FUNCTION_DECLARATION
            | kind::GENERATOR_FUNCTION_DECLARATION
            | kind::FUNCTION_EXPRESSION
            | kind::GENERATOR_FUNCTION
            | kind::ARROW_FUNCTION
            | kind::METHOD_DEFINITION
    )
}

impl<'a, 't> Extractor<'a, 't> {
    fn specifier(&mut self, node: Node<'_>) -> Option<String> {
        let text = string_content(node, self.src);
        if text.len() > MAX_SPECIFIER_BYTES {
            self.sink.note(
                DiagSeverity::Warning,
                DiagCode::UnsupportedConstruct,
                "module specifier too long; skipped",
                Some(self.src.range(node)),
            );
            return None;
        }
        Some(text)
    }

    fn push_import(&mut self, import: IrImport, register: bool) {
        let index = u32::try_from(self.out.imports.len()).unwrap_or(u32::MAX);
        if register {
            for (i, b) in import.bindings.iter().enumerate() {
                let binding = BindingRef {
                    import: index,
                    binding: u32::try_from(i).unwrap_or(u32::MAX),
                };
                self.out.bindings.insert(&b.local, binding);
            }
        }
        self.out.imports.push(import);
    }

    // ---- ESM imports ----

    fn esm_import(&mut self, stmt: Node<'t>) {
        let Some(source) = stmt.child_by_field_name(field::SOURCE) else {
            return;
        };
        let Some(specifier) = self.specifier(source) else {
            return;
        };
        let type_only = has_child_kind(stmt, "type");
        let mut bindings = Vec::new();
        let mut import_kind = ImportKind::SideEffect;
        let mut cursor = stmt.walk();
        for child in stmt.named_children(&mut cursor) {
            match child.kind() {
                kind::IMPORT_CLAUSE => {
                    import_kind = ImportKind::Esm;
                    self.import_clause(child, type_only, &mut bindings);
                }
                kind::IMPORT_REQUIRE_CLAUSE => {
                    import_kind = ImportKind::ImportEquals;
                    if let Some(id) = child.named_child(0) {
                        if id.kind() == kind::IDENTIFIER {
                            bindings.push(ImportBinding {
                                local: self.src.text(id).into_owned(),
                                imported: Imported::CjsModule,
                                type_only,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        let import = IrImport {
            specifier,
            kind: import_kind,
            type_only,
            bindings,
            range: self.src.range(stmt),
        };
        self.push_import(import, true);
    }

    fn import_clause(&mut self, clause: Node<'t>, type_only: bool, out: &mut Vec<ImportBinding>) {
        let mut cursor = clause.walk();
        for part in clause.named_children(&mut cursor) {
            match part.kind() {
                kind::IDENTIFIER => out.push(ImportBinding {
                    local: self.src.text(part).into_owned(),
                    imported: Imported::Default,
                    type_only,
                }),
                kind::NAMESPACE_IMPORT => {
                    let mut c = part.walk();
                    let id = part
                        .named_children(&mut c)
                        .find(|n| n.kind() == kind::IDENTIFIER);
                    if let Some(id) = id {
                        out.push(ImportBinding {
                            local: self.src.text(id).into_owned(),
                            imported: Imported::Namespace,
                            type_only,
                        });
                    }
                }
                kind::NAMED_IMPORTS => {
                    let mut c = part.walk();
                    for spec in part.named_children(&mut c) {
                        if spec.kind() != kind::IMPORT_SPECIFIER {
                            continue;
                        }
                        let Some(name) = spec.child_by_field_name(field::NAME) else {
                            continue;
                        };
                        let name_text = string_or_text(name, self.src);
                        let local = spec
                            .child_by_field_name(field::ALIAS)
                            .map(|a| self.src.text(a).into_owned())
                            .unwrap_or_else(|| name_text.clone());
                        let imported = if name_text == "default" {
                            Imported::Default
                        } else {
                            Imported::Named(name_text)
                        };
                        out.push(ImportBinding {
                            local,
                            imported,
                            type_only: type_only || has_child_kind(spec, "type"),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // ---- ESM exports ----

    fn local_by_name(&self, name: &str) -> Option<LocalId> {
        self.table
            .top_level
            .get(name)
            .and_then(|ids| ids.first())
            .copied()
    }

    fn export_statement(&mut self, stmt: Node<'t>) {
        let type_only = has_child_kind(stmt, "type");
        let source_node = stmt.child_by_field_name(field::SOURCE);
        let source = source_node.and_then(|s| self.specifier(s));
        let mut cursor = stmt.walk();
        let children: Vec<Node<'t>> = stmt.children(&mut cursor).collect();

        // `export * from 'x'`
        if children.iter().any(|c| c.kind() == "*") {
            if let Some(specifier) = source {
                self.out.exports.push(IrExport::StarReexport {
                    specifier,
                    as_namespace: None,
                    type_only,
                });
            }
            return;
        }
        for child in &children {
            match child.kind() {
                // `export * as ns from 'x'`
                kind::NAMESPACE_EXPORT => {
                    let mut c = child.walk();
                    let id = child.named_children(&mut c).next();
                    if let (Some(specifier), Some(id)) = (source.clone(), id) {
                        self.out.exports.push(IrExport::StarReexport {
                            specifier,
                            as_namespace: Some(string_or_text(id, self.src)),
                            type_only,
                        });
                    }
                    return;
                }
                kind::EXPORT_CLAUSE => {
                    self.export_clause(*child, source.as_deref(), source_node.is_some(), type_only);
                    return;
                }
                _ => {}
            }
        }
        // `export = X`
        if children.iter().any(|c| c.kind() == "=") {
            let mut c = stmt.walk();
            let id = stmt
                .named_children(&mut c)
                .find(|n| n.kind() == kind::IDENTIFIER);
            let symbol = id.and_then(|id| self.local_by_name(&self.src.text(id)));
            self.out.exports.push(IrExport::ExportAssignment { symbol });
            return;
        }
        let is_default = children.iter().any(|c| c.kind() == "default");
        if let Some(decl) = stmt.child_by_field_name(field::DECLARATION) {
            self.export_declaration(stmt, decl, is_default, type_only);
        } else if is_default {
            if let Some(value) = stmt.child_by_field_name(field::VALUE) {
                let expr = to_ir_expr(value, self.src);
                self.out.exports.push(IrExport::DefaultExpr { expr });
            }
        }
    }

    fn export_declaration(
        &mut self,
        stmt: Node<'t>,
        decl: Node<'t>,
        is_default: bool,
        type_only: bool,
    ) {
        let range = self.src.range(stmt);
        let mut found = false;
        for symbol in &self.table.symbols {
            if symbol.parent != Some(LocalId(0))
                || !symbol.modifiers.contains(Modifiers::EXPORTED)
                || symbol.range.start < range.start
                || symbol.range.end > range.end
            {
                continue;
            }
            found = true;
            let exported_as = if is_default || symbol.modifiers.contains(Modifiers::DEFAULT_EXPORT)
            {
                "default".to_owned()
            } else {
                symbol.name.clone()
            };
            self.out.exports.push(IrExport::Local {
                symbol: symbol.local_id,
                exported_as,
                type_only,
            });
        }
        if !found && is_default {
            let expr = to_ir_expr(decl, self.src);
            self.out.exports.push(IrExport::DefaultExpr { expr });
        }
    }

    fn export_clause(
        &mut self,
        clause: Node<'t>,
        source: Option<&str>,
        has_source: bool,
        type_only: bool,
    ) {
        if has_source && source.is_none() {
            return;
        }
        let mut cursor = clause.walk();
        for spec in clause.named_children(&mut cursor) {
            if spec.kind() != kind::EXPORT_SPECIFIER {
                continue;
            }
            let Some(name_node) = spec.child_by_field_name(field::NAME) else {
                continue;
            };
            let name = string_or_text(name_node, self.src);
            let exported_as = spec
                .child_by_field_name(field::ALIAS)
                .map(|a| string_or_text(a, self.src))
                .unwrap_or_else(|| name.clone());
            let type_only = type_only || has_child_kind(spec, "type");
            if let Some(specifier) = source {
                let imported = if name == "default" {
                    Imported::Default
                } else {
                    Imported::Named(name)
                };
                self.out.exports.push(IrExport::Reexport {
                    specifier: specifier.to_owned(),
                    imported,
                    exported_as,
                    type_only,
                });
            } else if let Some(symbol) = self.local_by_name(&name) {
                self.out.exports.push(IrExport::Local {
                    symbol,
                    exported_as,
                    type_only,
                });
            } else if let Some(binding) = self.out.bindings.get(&name) {
                // An imported binding exported again: a re-export of what it imported.
                let found = self.out.imports.get(binding.import as usize).and_then(|i| {
                    i.bindings
                        .get(binding.binding as usize)
                        .map(|b| (i.specifier.clone(), b.imported.clone(), b.type_only))
                });
                if let Some((specifier, imported, binding_type_only)) = found {
                    self.out.exports.push(IrExport::Reexport {
                        specifier,
                        imported,
                        exported_as,
                        type_only: type_only || binding_type_only,
                    });
                }
            }
        }
    }

    // ---- CommonJS exports (module level only) ----

    fn symbol_of_identifier(&self, node: Node<'_>) -> Option<LocalId> {
        if node.kind() == kind::IDENTIFIER {
            self.local_by_name(&self.src.text(node))
        } else {
            None
        }
    }

    fn cjs_export_statement(&mut self, stmt: Node<'t>) {
        let Some(assign) = stmt.named_child(0) else {
            return;
        };
        if assign.kind() != kind::ASSIGNMENT_EXPRESSION {
            return;
        }
        let (Some(left), Some(right)) = (
            assign.child_by_field_name(field::LEFT),
            assign.child_by_field_name(field::RIGHT),
        ) else {
            return;
        };
        if left.kind() != kind::MEMBER_EXPRESSION {
            return;
        }
        let text = self.src.text(left).into_owned();
        if text == "module.exports" {
            let expr = to_ir_expr(right, self.src);
            let symbol = self.symbol_of_identifier(right);
            self.out.exports.push(IrExport::CjsModuleExports {
                symbol,
                expr: expr.clone(),
            });
            if right.kind() == kind::OBJECT {
                if let IrExpr::Object(pairs) = expr {
                    for (name, value) in pairs {
                        if name == "..." {
                            continue;
                        }
                        let symbol = match &value {
                            IrExpr::Ident(id) => self.local_by_name(id),
                            _ => None,
                        };
                        self.out
                            .exports
                            .push(IrExport::CjsExportsProperty { name, symbol });
                    }
                }
            }
            return;
        }
        let name = text
            .strip_prefix("module.exports.")
            .or_else(|| text.strip_prefix("exports."));
        if let Some(name) = name {
            if !name.contains('.') {
                let symbol = self.symbol_of_identifier(right);
                self.out.exports.push(IrExport::CjsExportsProperty {
                    name: name.to_owned(),
                    symbol,
                });
            }
        }
    }

    // ---- require / dynamic import, any scope ----

    /// `Some(Some(spec))` for `require('lit')`, `Some(None)` for a non-literal argument or an
    /// over-long specifier, `None` when `call` is not a `require` call.
    fn require_literal(&mut self, call: Node<'t>) -> Option<Option<String>> {
        let function = call.child_by_field_name(field::FUNCTION)?;
        if function.kind() != kind::IDENTIFIER || self.src.text(function) != "require" {
            return None;
        }
        let args = call.child_by_field_name(field::ARGUMENTS)?;
        let first = args.named_child(0)?;
        if first.kind() == kind::STRING {
            Some(self.specifier(first))
        } else {
            Some(None)
        }
    }

    fn walk(&mut self, node: Node<'t>, depth: usize, in_fn: bool, shadowed: bool) {
        if depth > MAX_VISIT_DEPTH {
            depth_limit(self.sink, self.src.range(node));
            return;
        }
        let mut in_fn = in_fn;
        let mut shadowed = shadowed;
        let k = node.kind();
        if is_function_like(k) {
            in_fn = true;
            if let Some(params) = node.child_by_field_name(field::PARAMETERS) {
                if declares_require(params, self.src) {
                    shadowed = true;
                }
            }
        }
        let require_ok = !shadowed && !self.out.bindings.contains("require");
        match k {
            kind::VARIABLE_DECLARATOR if require_ok => self.cjs_declarator(node, !in_fn),
            kind::EXPRESSION_STATEMENT if require_ok => {
                if let Some(call) = node.named_child(0) {
                    if call.kind() == kind::CALL_EXPRESSION {
                        if let Some(Some(specifier)) = self.require_literal(call) {
                            self.push_import(
                                IrImport {
                                    specifier,
                                    kind: ImportKind::CjsRequire,
                                    type_only: false,
                                    bindings: Vec::new(),
                                    range: self.src.range(call),
                                },
                                false,
                            );
                        }
                    }
                }
            }
            kind::CALL_EXPRESSION => self.dynamic_import(node),
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, depth + 1, in_fn, shadowed);
        }
    }

    fn dynamic_import(&mut self, call: Node<'t>) {
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            return;
        };
        if function.kind() != kind::IMPORT {
            return;
        }
        let first = call
            .child_by_field_name(field::ARGUMENTS)
            .and_then(|a| a.named_child(0));
        let literal = first.and_then(|a| match a.kind() {
            kind::STRING => Some(string_content(a, self.src)),
            kind::TEMPLATE_STRING if !has_child_kind(a, kind::TEMPLATE_SUBSTITUTION) => {
                Some(self.src.text(a).trim_matches('`').to_owned())
            }
            _ => None,
        });
        match literal {
            Some(specifier) if specifier.len() <= MAX_SPECIFIER_BYTES => {
                self.push_import(
                    IrImport {
                        specifier,
                        kind: ImportKind::Dynamic,
                        type_only: false,
                        bindings: Vec::new(),
                        range: self.src.range(call),
                    },
                    false,
                );
            }
            Some(_) => {}
            None => self.sink.note(
                DiagSeverity::Info,
                DiagCode::DynamicImportNonLiteral,
                "dynamic import with a non-literal specifier",
                Some(self.src.range(call)),
            ),
        }
    }

    fn cjs_declarator(&mut self, decl: Node<'t>, module_level: bool) {
        let (Some(name), Some(value)) = (
            decl.child_by_field_name(field::NAME),
            decl.child_by_field_name(field::VALUE),
        ) else {
            return;
        };
        let mut value = value;
        while matches!(
            value.kind(),
            kind::AWAIT_EXPRESSION | kind::PARENTHESIZED_EXPRESSION
        ) {
            match value.named_child(0) {
                Some(inner) => value = inner,
                None => return,
            }
        }
        // `require('x').prop` or `require('x')`.
        let (call, member) = if value.kind() == kind::MEMBER_EXPRESSION {
            let object = value.child_by_field_name(field::OBJECT);
            let property = value.child_by_field_name(field::PROPERTY);
            match (object, property) {
                (Some(o), Some(p)) if o.kind() == kind::CALL_EXPRESSION => (o, Some(p)),
                _ => return,
            }
        } else if value.kind() == kind::CALL_EXPRESSION {
            (value, None)
        } else {
            return;
        };
        let specifier = match self.require_literal(call) {
            Some(Some(s)) => s,
            Some(None) => {
                self.sink.note(
                    DiagSeverity::Info,
                    DiagCode::UnsupportedConstruct,
                    "require with a non-literal argument",
                    Some(self.src.range(call)),
                );
                return;
            }
            None => return,
        };
        let mut bindings = Vec::new();
        match (name.kind(), member) {
            (kind::IDENTIFIER, Some(prop)) => bindings.push(ImportBinding {
                local: self.src.text(name).into_owned(),
                imported: Imported::Named(self.src.text(prop).into_owned()),
                type_only: false,
            }),
            (kind::IDENTIFIER, None) => bindings.push(ImportBinding {
                local: self.src.text(name).into_owned(),
                imported: Imported::CjsModule,
                type_only: false,
            }),
            (kind::OBJECT_PATTERN, None) => {
                let mut cursor = name.walk();
                for part in name.named_children(&mut cursor) {
                    match part.kind() {
                        kind::SHORTHAND_PROPERTY_IDENTIFIER_PATTERN => {
                            let n = self.src.text(part).into_owned();
                            bindings.push(ImportBinding {
                                local: n.clone(),
                                imported: Imported::Named(n),
                                type_only: false,
                            });
                        }
                        kind::PAIR_PATTERN => {
                            let key = part.child_by_field_name(field::KEY);
                            let val = part.child_by_field_name(field::VALUE);
                            if let (Some(key), Some(val)) = (key, val) {
                                if val.kind() == kind::IDENTIFIER {
                                    bindings.push(ImportBinding {
                                        local: self.src.text(val).into_owned(),
                                        imported: Imported::Named(string_or_text(key, self.src)),
                                        type_only: false,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        self.push_import(
            IrImport {
                specifier,
                kind: ImportKind::CjsRequire,
                type_only: false,
                bindings,
                range: self.src.range(decl),
            },
            module_level,
        );
    }
}

/// Text of a name node; string-literal names (`export { "a-b" as c }`) lose their quotes.
fn string_or_text(node: Node<'_>, src: &Source<'_>) -> String {
    if node.kind() == kind::STRING {
        string_content(node, src)
    } else {
        src.text(node).into_owned()
    }
}

/// Whether a parameter list binds a name `require`.
fn declares_require(params: Node<'_>, src: &Source<'_>) -> bool {
    let mut cursor = params.walk();
    let found = params.named_children(&mut cursor).any(|p| {
        let pattern = p.child_by_field_name(field::PATTERN).unwrap_or(p);
        pattern.kind() == kind::IDENTIFIER && src.text(pattern) == "require"
    });
    found
}
