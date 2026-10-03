//! Declaration extraction (TSA-003): classes, interfaces, type aliases, enums, functions,
//! methods, properties, variables, namespaces and default exports.
//!
//! The walk is a bounded recursive descent over statement lists, class bodies and object
//! literals. Function bodies are not entered (local declarations are not symbols), so the
//! recursion depth is bounded by declaration nesting, which is additionally capped at
//! [`MAX_VISIT_DEPTH`].

use std::collections::BTreeMap;

use analysis_ir::{
    AnalyzerConfig, AnonymousFnPolicy, AttrValue, ConstValue, DiagCode, DiagSeverity, IrDecorator,
    IrParam, IrSymbol, LocalId, Modifiers, ParamProperty, QualifiedName, Visibility,
};
use review_core::location::{Position, SourceRange};
use review_core::symbol::SymbolKind;
use tree_sitter::Node;

use super::expr::{
    collapse_ws, key_text, member_chain, string_content, to_ir_expr, truncate_chars,
};
use super::{depth_limit, MAX_VISIT_DEPTH};
use crate::diagnostics::DiagnosticSink;
use crate::kinds::{field, kind};
use crate::naming::{qualify, Construct, MemberName, NameDecision};
use crate::text::Source;

const MAX_SIGNATURE_CHARS: usize = 512;
const MAX_CONST_STRING_BYTES: usize = 1024;

/// The tree nodes behind a symbol, kept for the later passes (references, facts, hashes).
#[derive(Debug, Clone, Copy)]
pub struct SymNodes<'t> {
    /// The whole declaration (including wrappers and decorators where available).
    pub node: Node<'t>,
    /// Function/method body, class/interface/enum body, or initializer expression.
    pub body: Option<Node<'t>>,
    pub params: Option<Node<'t>>,
}

/// Symbols plus the nodes they came from.
#[derive(Debug)]
pub struct Table<'t> {
    pub symbols: Vec<IrSymbol>,
    pub nodes: Vec<SymNodes<'t>>,
    /// Tree node id of the declaring node -> symbol.
    pub by_node: BTreeMap<usize, LocalId>,
    /// Module-level symbols by simple name (several when declarations merge).
    pub top_level: BTreeMap<String, Vec<LocalId>>,
}

#[derive(Clone)]
struct Scope {
    parent: LocalId,
    /// Qualified name of the parent, `None` at module level.
    qn: Option<QualifiedName>,
    ambient: bool,
    depth: usize,
}

/// Declaration context carried from wrapper statements to the declaration inside.
#[derive(Clone, Default)]
struct StmtCtx<'t> {
    exported: bool,
    default: bool,
    ambient: bool,
    /// Outermost wrapper node (export statement), used for the range.
    wrapper: Option<Node<'t>>,
    decorators: Vec<IrDecorator>,
    decorator_start: Option<Node<'t>>,
    overloads: Vec<String>,
}

struct Walker<'a, 't> {
    src: &'a Source<'a>,
    cfg: &'a AnalyzerConfig,
    sink: &'a mut DiagnosticSink,
    table: Table<'t>,
}

/// Extracts every declaration below `root`. `module` becomes `symbols[0]`.
pub fn collect<'a, 't>(
    root: Node<'t>,
    src: &'a Source<'a>,
    cfg: &'a AnalyzerConfig,
    sink: &'a mut DiagnosticSink,
    module: IrSymbol,
) -> Table<'t> {
    let mut walker = Walker {
        src,
        cfg,
        sink,
        table: Table {
            symbols: vec![module],
            nodes: vec![SymNodes {
                node: root,
                body: None,
                params: None,
            }],
            by_node: BTreeMap::new(),
            top_level: BTreeMap::new(),
        },
    };
    let scope = Scope {
        parent: LocalId(0),
        qn: None,
        ambient: false,
        depth: 0,
    };
    walker.visit_statements(root, &scope);
    if cfg.anonymous_functions == AnonymousFnPolicy::Emit {
        walker.emit_anonymous();
    }
    walker.table
}

fn is_secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "apikey",
        "api_key",
        "api-key",
        "privatekey",
        "private_key",
        "private-key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

struct FnInfo {
    name: String,
    is_signature: bool,
}

impl<'a, 't> Walker<'a, 't> {
    // ----- helpers ---------------------------------------------------------------------

    fn text(&self, node: Node<'t>) -> String {
        self.src.text(node).into_owned()
    }

    fn range(&self, node: Node<'t>) -> SourceRange {
        self.src.range(node)
    }

    fn range_between(&self, start: Node<'t>, end: Node<'t>) -> SourceRange {
        let a = self.src.position(start.start_position());
        let b = self.src.position(end.end_position());
        SourceRange::new(a, b).unwrap_or_else(|_| self.range(end))
    }

    fn type_text(&self, annotation: Node<'t>) -> Option<String> {
        // `type_annotation` is `: T`; other nodes are the type itself.
        let inner = if annotation.kind() == kind::TYPE_ANNOTATION {
            annotation.named_child(0)?
        } else {
            annotation
        };
        let text = collapse_ws(&self.src.text(inner));
        (!text.is_empty()).then_some(truncate_chars(&text, 512))
    }

    fn type_params(&self, node: Node<'t>) -> Vec<String> {
        let Some(params) = node.child_by_field_name(field::TYPE_PARAMETERS) else {
            return Vec::new();
        };
        let mut cursor = params.walk();
        params
            .named_children(&mut cursor)
            .filter(|c| c.kind() == kind::TYPE_PARAMETER)
            .map(|c| truncate_chars(&collapse_ws(&self.src.text(c)), 200))
            .collect()
    }

    fn has_token(&self, node: Node<'t>, token: &str) -> bool {
        let mut cursor = node.walk();
        let found = node.children(&mut cursor).any(|c| c.kind() == token);
        found
    }

    fn accessibility(&self, node: Node<'t>) -> Option<Visibility> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == kind::ACCESSIBILITY_MODIFIER {
                return match self.src.text(child).trim() {
                    "private" => Some(Visibility::Private),
                    "protected" => Some(Visibility::Protected),
                    "public" => Some(Visibility::Public),
                    _ => None,
                };
            }
        }
        None
    }

    fn member_modifiers(&self, node: Node<'t>) -> Modifiers {
        let mut m = Modifiers::empty();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "static" => m.insert(Modifiers::STATIC),
                "async" => m.insert(Modifiers::ASYNC),
                "readonly" => m.insert(Modifiers::READONLY),
                "abstract" => m.insert(Modifiers::ABSTRACT),
                "declare" => m.insert(Modifiers::DECLARE),
                "*" => m.insert(Modifiers::GENERATOR),
                "?" => m.insert(Modifiers::OPTIONAL),
                kind::OVERRIDE_MODIFIER => m.insert(Modifiers::OVERRIDE),
                _ => {}
            }
        }
        m
    }

    fn decorators_of(&self, node: Node<'t>) -> Vec<(IrDecorator, Node<'t>)> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|c| c.kind() == kind::DECORATOR)
            .filter_map(|d| self.decorator(d).map(|ir| (ir, d)))
            .collect()
    }

    fn decorator(&self, node: Node<'t>) -> Option<IrDecorator> {
        let inner = node.named_child(0)?;
        let (name, args) = match inner.kind() {
            kind::CALL_EXPRESSION => {
                let callee = inner.child_by_field_name(field::FUNCTION)?;
                let name = member_chain(callee, self.src)?.join(".");
                let mut args = Vec::new();
                if let Some(arguments) = inner.child_by_field_name(field::ARGUMENTS) {
                    let mut cursor = arguments.walk();
                    for arg in arguments.named_children(&mut cursor) {
                        if arg.kind() != kind::COMMENT {
                            args.push(to_ir_expr(arg, self.src));
                        }
                    }
                }
                (name, args)
            }
            _ => (member_chain(inner, self.src)?.join("."), Vec::new()),
        };
        Some(IrDecorator {
            name,
            args,
            range: self.range(node),
        })
    }

    fn params(&self, node: Node<'t>) -> (Vec<IrParam>, Option<Node<'t>>) {
        let list = node
            .child_by_field_name(field::PARAMETERS)
            .or_else(|| node.child_by_field_name(field::PARAMETER));
        let Some(list) = list else {
            return (Vec::new(), None);
        };
        if list.kind() != kind::FORMAL_PARAMETERS {
            // `x => ...`: a single unparenthesized parameter.
            let name = self.text(list);
            return (
                vec![IrParam {
                    name,
                    type_text: None,
                    optional: false,
                    rest: false,
                    decorators: Vec::new(),
                    property: None,
                }],
                Some(list),
            );
        }
        let mut params = Vec::new();
        let mut cursor = list.walk();
        for p in list.named_children(&mut cursor) {
            if !matches!(
                p.kind(),
                kind::REQUIRED_PARAMETER | kind::OPTIONAL_PARAMETER
            ) {
                continue;
            }
            let pattern = p.child_by_field_name(field::PATTERN);
            let (name, rest) = match pattern {
                Some(pat) if pat.kind() == kind::REST_PATTERN => (
                    pat.named_child(0).map(|n| self.text(n)).unwrap_or_default(),
                    true,
                ),
                Some(pat) if pat.kind() == kind::IDENTIFIER || pat.kind() == kind::THIS => {
                    (self.text(pat), false)
                }
                Some(pat) => (truncate_chars(&collapse_ws(&self.text(pat)), 120), false),
                None => (String::new(), false),
            };
            let type_text = p
                .child_by_field_name(field::TYPE)
                .and_then(|t| self.type_text(t));
            let accessibility = self.accessibility(p);
            let readonly = self.has_token(p, "readonly");
            let property = (accessibility.is_some() || readonly).then_some(ParamProperty {
                visibility: accessibility.unwrap_or(Visibility::Public),
                readonly,
            });
            params.push(IrParam {
                name,
                type_text,
                optional: p.kind() == kind::OPTIONAL_PARAMETER || self.has_token(p, "?"),
                rest,
                decorators: self.decorators_of(p).into_iter().map(|(d, _)| d).collect(),
                property,
            });
        }
        (params, Some(list))
    }

    fn signature(
        &self,
        modifiers: Modifiers,
        name: &str,
        type_params: &[String],
        params: &[IrParam],
        return_type: Option<&str>,
    ) -> String {
        let mut out = String::new();
        if modifiers.contains(Modifiers::ASYNC) {
            out.push_str("async ");
        }
        out.push_str(name);
        if !type_params.is_empty() {
            out.push('<');
            out.push_str(&type_params.join(", "));
            out.push('>');
        }
        out.push('(');
        let rendered: Vec<String> = params
            .iter()
            .map(|p| {
                let mut s = String::new();
                if p.rest {
                    s.push_str("...");
                }
                s.push_str(&p.name);
                if p.optional {
                    s.push('?');
                }
                if let Some(t) = &p.type_text {
                    s.push_str(": ");
                    s.push_str(t);
                }
                s
            })
            .collect();
        out.push_str(&rendered.join(", "));
        out.push(')');
        if let Some(ret) = return_type {
            out.push_str(": ");
            out.push_str(ret);
        }
        truncate_chars(&collapse_ws(&out), MAX_SIGNATURE_CHARS)
    }

    /// Display signature of a signature-only node (overload / method signature).
    fn signature_of_node(&self, node: Node<'t>, name: &str) -> String {
        let modifiers = self.member_modifiers(node);
        let tparams = self.type_params(node);
        let (params, _) = self.params(node);
        let ret = node
            .child_by_field_name(field::RETURN_TYPE)
            .and_then(|r| self.type_text(r));
        self.signature(modifiers, name, &tparams, &params, ret.as_deref())
    }

    // ----- symbol creation ---------------------------------------------------------------

    fn add(
        &mut self,
        scope: &Scope,
        kind: SymbolKind,
        decision: NameDecision,
        range: SourceRange,
        nodes: SymNodes<'t>,
        decl_node: Node<'t>,
    ) -> Option<LocalId> {
        let qn = match decision {
            NameDecision::Named(qn) => qn,
            NameDecision::Skip(code) => {
                self.sink.note(
                    DiagSeverity::Info,
                    code,
                    "declaration skipped: it has no usable name",
                    Some(range),
                );
                return None;
            }
            NameDecision::Fold => return None,
        };
        let id = LocalId(self.table.symbols.len() as u32);
        let name = qn.last().cloned().unwrap_or_default();
        let mut symbol = IrSymbol::new(id, kind, name.clone(), qn, range);
        symbol.parent = Some(scope.parent);
        if scope.ambient {
            symbol.modifiers.insert(Modifiers::AMBIENT);
        }
        self.table.symbols.push(symbol);
        self.table.nodes.push(nodes);
        self.table.by_node.insert(decl_node.id(), id);
        if scope.parent == LocalId(0) {
            self.table.top_level.entry(name).or_default().push(id);
        }
        Some(id)
    }

    fn sym(&mut self, id: LocalId) -> &mut IrSymbol {
        &mut self.table.symbols[id.0 as usize]
    }

    fn child_scope(&self, parent: LocalId, scope: &Scope, ambient: bool) -> Scope {
        Scope {
            parent,
            qn: Some(self.table.symbols[parent.0 as usize].qualified_name.clone()),
            ambient: scope.ambient || ambient,
            depth: scope.depth + 1,
        }
    }

    // ----- statements --------------------------------------------------------------------

    fn visit_statements(&mut self, container: Node<'t>, scope: &Scope) {
        if scope.depth >= MAX_VISIT_DEPTH {
            depth_limit(self.sink, self.range(container));
            return;
        }
        let children: Vec<Node<'t>> = {
            let mut cursor = container.walk();
            container
                .named_children(&mut cursor)
                .filter(|c| c.kind() != kind::COMMENT)
                .collect()
        };
        let infos: Vec<Option<FnInfo>> = children.iter().map(|c| self.fn_info(*c)).collect();
        let mut i = 0;
        while i < children.len() {
            let stmt = children[i];
            if let Some(info) = &infos[i] {
                if info.is_signature {
                    // Collect the run of overload signatures with the same name.
                    let mut j = i;
                    let mut sigs = Vec::new();
                    while j < children.len() {
                        match &infos[j] {
                            Some(next) if next.is_signature && next.name == info.name => {
                                sigs.push(children[j]);
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    let implementation = match infos.get(j) {
                        Some(Some(next)) if !next.is_signature && next.name == info.name => {
                            Some(children[j])
                        }
                        _ => None,
                    };
                    let signatures: Vec<String> = sigs
                        .iter()
                        .map(|s| self.function_signature_text(*s))
                        .collect();
                    match implementation {
                        Some(imp) => {
                            self.visit_stmt(imp, scope, signatures);
                            i = j + 1;
                        }
                        None => {
                            // Ambient: the first signature is the symbol, the rest overloads.
                            let rest = signatures.into_iter().skip(1).collect();
                            self.visit_stmt(sigs[0], scope, rest);
                            i = j;
                        }
                    }
                    continue;
                }
            }
            self.visit_stmt(stmt, scope, Vec::new());
            i += 1;
        }
    }

    /// Name and signature-ness of a function statement, looking through export/declare wrappers.
    fn fn_info(&self, stmt: Node<'t>) -> Option<FnInfo> {
        let mut node = stmt;
        loop {
            match node.kind() {
                kind::EXPORT_STATEMENT => node = node.child_by_field_name(field::DECLARATION)?,
                kind::AMBIENT_DECLARATION => {
                    let mut cursor = node.walk();
                    let inner = node
                        .named_children(&mut cursor)
                        .find(|c| c.kind() != kind::COMMENT)?;
                    node = inner;
                }
                kind::FUNCTION_SIGNATURE => {
                    let name = self.text(node.child_by_field_name(field::NAME)?);
                    return Some(FnInfo {
                        name,
                        is_signature: true,
                    });
                }
                kind::FUNCTION_DECLARATION | kind::GENERATOR_FUNCTION_DECLARATION => {
                    let name = self.text(node.child_by_field_name(field::NAME)?);
                    return Some(FnInfo {
                        name,
                        is_signature: false,
                    });
                }
                _ => return None,
            }
        }
    }

    fn function_signature_text(&self, stmt: Node<'t>) -> String {
        let mut node = stmt;
        while matches!(
            node.kind(),
            kind::EXPORT_STATEMENT | kind::AMBIENT_DECLARATION
        ) {
            let next = node.child_by_field_name(field::DECLARATION).or_else(|| {
                let mut cursor = node.walk();
                let found = node
                    .named_children(&mut cursor)
                    .find(|c| c.kind() != kind::COMMENT && c.kind() != kind::DECORATOR);
                found
            });
            match next {
                Some(n) => node = n,
                None => break,
            }
        }
        let name = node
            .child_by_field_name(field::NAME)
            .map(|n| self.text(n))
            .unwrap_or_default();
        self.signature_of_node(node, &name)
    }

    fn visit_stmt(&mut self, node: Node<'t>, scope: &Scope, overloads: Vec<String>) {
        let ctx = StmtCtx {
            overloads,
            ..StmtCtx::default()
        };
        self.dispatch(node, scope, ctx);
    }

    fn dispatch(&mut self, node: Node<'t>, scope: &Scope, mut ctx: StmtCtx<'t>) {
        match node.kind() {
            kind::EXPORT_STATEMENT => self.visit_export(node, scope, ctx),
            kind::AMBIENT_DECLARATION => self.visit_ambient(node, scope, ctx),
            kind::EXPRESSION_STATEMENT => {
                // `namespace A.B {}` parses as an expression statement wrapping the module.
                let mut cursor = node.walk();
                let inner = node
                    .named_children(&mut cursor)
                    .find(|c| matches!(c.kind(), kind::INTERNAL_MODULE | kind::MODULE));
                if let Some(inner) = inner {
                    ctx.wrapper.get_or_insert(node);
                    self.visit_namespace(inner, scope, ctx);
                }
            }
            kind::INTERNAL_MODULE | kind::MODULE => self.visit_namespace(node, scope, ctx),
            kind::CLASS_DECLARATION | kind::ABSTRACT_CLASS_DECLARATION => {
                self.visit_class(node, scope, ctx, None);
            }
            kind::INTERFACE_DECLARATION => self.visit_interface(node, scope, ctx),
            kind::TYPE_ALIAS_DECLARATION => self.visit_type_alias(node, scope, ctx),
            kind::ENUM_DECLARATION => self.visit_enum(node, scope, ctx),
            kind::FUNCTION_DECLARATION
            | kind::GENERATOR_FUNCTION_DECLARATION
            | kind::FUNCTION_SIGNATURE => self.visit_function(node, scope, ctx),
            kind::LEXICAL_DECLARATION | kind::VARIABLE_DECLARATION => {
                self.visit_variables(node, scope, ctx);
            }
            kind::ERROR => {
                // Declarations inside error regions are still extracted.
                self.visit_statements(node, scope);
            }
            _ => {}
        }
    }

    fn visit_export(&mut self, node: Node<'t>, scope: &Scope, mut ctx: StmtCtx<'t>) {
        ctx.exported = true;
        ctx.wrapper = Some(node);
        let decorators = self.decorators_of(node);
        ctx.decorator_start = decorators.first().map(|(_, n)| *n);
        ctx.decorators = decorators.into_iter().map(|(d, _)| d).collect();
        let mut is_default = false;
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "default" {
                    is_default = true;
                }
            }
        }
        ctx.default = is_default;
        if let Some(decl) = node.child_by_field_name(field::DECLARATION) {
            let overloads = std::mem::take(&mut ctx.overloads);
            ctx.overloads = overloads;
            self.dispatch(decl, scope, ctx);
            return;
        }
        if is_default {
            if let Some(value) = node.child_by_field_name(field::VALUE) {
                self.visit_default_value(node, value, scope, ctx);
            }
        }
        // `export { a }`, `export * from`, `export = x` carry no declarations (TSA-004).
    }

    fn visit_ambient(&mut self, node: Node<'t>, scope: &Scope, mut ctx: StmtCtx<'t>) {
        ctx.ambient = true;
        ctx.wrapper.get_or_insert(node);
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        let is_global = {
            let mut c2 = node.walk();
            let found = node.children(&mut c2).any(|c| c.kind() == "global");
            found
        };
        if is_global {
            if let Some(block) = children.iter().find(|c| c.kind() == kind::STATEMENT_BLOCK) {
                let range = self.range(ctx.wrapper.unwrap_or(node));
                let decision = qualify(scope.qn.as_ref(), Construct::Declared("global".to_owned()));
                let nodes = SymNodes {
                    node,
                    body: Some(*block),
                    params: None,
                };
                if let Some(id) =
                    self.add(scope, SymbolKind::Namespace, decision, range, nodes, node)
                {
                    self.sym(id).modifiers.insert(Modifiers::DECLARE);
                    self.sym(id).body_range = Some(self.range(*block));
                    let inner = self.child_scope(id, scope, true);
                    self.visit_statements(*block, &inner);
                }
            }
            return;
        }
        for child in children {
            if child.kind() == kind::COMMENT {
                continue;
            }
            self.dispatch(child, scope, ctx.clone());
            break;
        }
    }

    // ----- default export values -----------------------------------------------------------

    fn visit_default_value(
        &mut self,
        export: Node<'t>,
        value: Node<'t>,
        scope: &Scope,
        ctx: StmtCtx<'t>,
    ) {
        let range = self.range(export);
        match value.kind() {
            kind::CLASS | kind::CLASS_DECLARATION => {
                self.visit_class(value, scope, ctx, Some(Construct::DefaultExport));
            }
            kind::FUNCTION_EXPRESSION | kind::GENERATOR_FUNCTION | kind::ARROW_FUNCTION => {
                let decision = qualify(scope.qn.as_ref(), Construct::DefaultExport);
                let nodes = SymNodes {
                    node: export,
                    body: value.child_by_field_name(field::BODY),
                    params: None,
                };
                if let Some(id) =
                    self.add(scope, SymbolKind::Function, decision, range, nodes, export)
                {
                    self.fill_function(id, value, &ctx, "default");
                    let sym = self.sym(id);
                    sym.modifiers.insert(Modifiers::DEFAULT_EXPORT);
                }
            }
            kind::OBJECT => {
                let decision = qualify(scope.qn.as_ref(), Construct::DefaultExport);
                let nodes = SymNodes {
                    node: export,
                    body: Some(value),
                    params: None,
                };
                if let Some(id) =
                    self.add(scope, SymbolKind::Constant, decision, range, nodes, export)
                {
                    {
                        let sym = self.sym(id);
                        sym.modifiers
                            .insert(Modifiers::EXPORTED | Modifiers::DEFAULT_EXPORT);
                    }
                    self.sym(id).body_range = Some(self.range(value));
                    let inner = self.child_scope(id, scope, false);
                    self.visit_object(value, &inner, 1);
                }
            }
            // `export default foo;` is an export of an existing binding (TSA-004).
            _ => {}
        }
    }

    // ----- namespaces ------------------------------------------------------------------

    fn visit_namespace(&mut self, node: Node<'t>, scope: &Scope, ctx: StmtCtx<'t>) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let segments: Vec<String> = match name_node.kind() {
            kind::STRING => vec![string_content(name_node, self.src)],
            kind::NESTED_IDENTIFIER => member_chain(name_node, self.src).unwrap_or_default(),
            _ => vec![self.text(name_node)],
        };
        let range = self.range(ctx.wrapper.unwrap_or(node));
        let body = node.child_by_field_name(field::BODY);
        let mut current = scope.clone();
        let mut last = None;
        for (index, segment) in segments.iter().enumerate() {
            let decision = qualify(current.qn.as_ref(), Construct::Declared(segment.clone()));
            let nodes = SymNodes {
                node,
                body,
                params: None,
            };
            // Nested symbols of one dotted declaration share the declaring node, so only the
            // outermost is registered in `by_node`.
            let id = self.add(
                &current,
                SymbolKind::Namespace,
                decision,
                range,
                nodes,
                node,
            );
            let Some(id) = id else { return };
            if index == 0 && ctx.exported {
                self.sym(id).modifiers.insert(Modifiers::EXPORTED);
            }
            if ctx.ambient || name_node.kind() == kind::STRING {
                self.sym(id).modifiers.insert(Modifiers::DECLARE);
            }
            if name_node.kind() == kind::STRING {
                self.sym(id).modifiers.insert(Modifiers::AMBIENT);
            }
            self.sym(id).name_range = Some(self.range(name_node));
            current = self.child_scope(
                id,
                &current,
                name_node.kind() == kind::STRING || ctx.ambient,
            );
            last = Some(id);
        }
        if let (Some(id), Some(body)) = (last, body) {
            self.sym(id).body_range = Some(self.range(body));
            self.visit_statements(body, &current);
        }
    }

    // ----- classes ---------------------------------------------------------------------

    fn heritage(&self, class: Node<'t>) -> (Vec<String>, Vec<String>) {
        let mut extends = Vec::new();
        let mut implements = Vec::new();
        let mut cursor = class.walk();
        for child in class.children(&mut cursor) {
            if child.kind() != kind::CLASS_HERITAGE {
                continue;
            }
            let mut inner = child.walk();
            for clause in child.children(&mut inner) {
                match clause.kind() {
                    kind::EXTENDS_CLAUSE => {
                        let mut parts = Vec::new();
                        let mut c3 = clause.walk();
                        for part in clause.named_children(&mut c3) {
                            if part.kind() != kind::COMMENT {
                                parts.push(self.text(part));
                            }
                        }
                        if let Some(value) = clause.child_by_field_name(field::VALUE) {
                            let mut text = collapse_ws(&self.text(value));
                            if let Some(args) = clause.child_by_field_name(field::TYPE_ARGUMENTS) {
                                text.push_str(&collapse_ws(&self.text(args)));
                            }
                            extends.push(truncate_chars(&text, 200));
                        } else if !parts.is_empty() {
                            extends.push(truncate_chars(&collapse_ws(&parts.join("")), 200));
                        }
                    }
                    kind::IMPLEMENTS_CLAUSE => {
                        let mut c3 = clause.walk();
                        for part in clause.named_children(&mut c3) {
                            if part.kind() != kind::COMMENT {
                                implements
                                    .push(truncate_chars(&collapse_ws(&self.text(part)), 200));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        (extends, implements)
    }

    fn visit_class(
        &mut self,
        node: Node<'t>,
        scope: &Scope,
        mut ctx: StmtCtx<'t>,
        name_override: Option<Construct>,
    ) {
        let name_node = node.child_by_field_name(field::NAME);
        let construct = match (name_override, name_node) {
            (Some(c), _) => c,
            (None, Some(n)) => Construct::Declared(self.text(n)),
            (None, None) => {
                self.sink.note(
                    DiagSeverity::Info,
                    DiagCode::UnsupportedConstruct,
                    "class without a name",
                    Some(self.range(node)),
                );
                return;
            }
        };
        let own_decorators = self.decorators_of(node);
        let wrapper = ctx.wrapper.unwrap_or(node);
        let range = match ctx
            .decorator_start
            .or_else(|| own_decorators.first().map(|(_, n)| *n))
        {
            Some(start) if start.start_byte() < wrapper.start_byte() => {
                self.range_between(start, wrapper)
            }
            _ => self.range(wrapper),
        };
        let body = node.child_by_field_name(field::BODY);
        let decision = qualify(scope.qn.as_ref(), construct);
        let nodes = SymNodes {
            node: wrapper,
            body,
            params: None,
        };
        let Some(id) = self.add(scope, SymbolKind::Class, decision, range, nodes, node) else {
            return;
        };
        self.table.by_node.insert(wrapper.id(), id);
        let (extends, implements) = self.heritage(node);
        let type_params = self.type_params(node);
        let abstract_class = node.kind() == kind::ABSTRACT_CLASS_DECLARATION;
        let mut decorators = std::mem::take(&mut ctx.decorators);
        decorators.extend(own_decorators.into_iter().map(|(d, _)| d));
        {
            let ctx_exported = ctx.exported;
            let ctx_default = ctx.default;
            let ctx_ambient = ctx.ambient;
            let sym = self.sym(id);
            if ctx_exported {
                sym.modifiers.insert(Modifiers::EXPORTED);
            }
            if ctx_default {
                sym.modifiers.insert(Modifiers::DEFAULT_EXPORT);
            }
            if ctx_ambient {
                sym.modifiers.insert(Modifiers::DECLARE);
            }
            if abstract_class {
                sym.modifiers.insert(Modifiers::ABSTRACT);
            }
            sym.heritage.extends = extends;
            sym.heritage.implements = implements;
            sym.type_params = type_params;
            sym.decorators = decorators;
        }
        self.sym(id).name_range = name_node.map(|n| self.range(n));
        self.sym(id).body_range = body.map(|b| self.range(b));
        let name = self.sym(id).name.clone();
        let sig = {
            let tp = self.sym(id).type_params.clone();
            let mut s = format!("class {name}");
            if !tp.is_empty() {
                s.push('<');
                s.push_str(&tp.join(", "));
                s.push('>');
            }
            truncate_chars(&collapse_ws(&s), MAX_SIGNATURE_CHARS)
        };
        self.sym(id).signature = Some(sig);
        if let Some(body) = body {
            let inner = self.child_scope(id, scope, false);
            self.visit_class_body(body, &inner);
        }
    }

    fn member_name(&mut self, name_node: Node<'t>) -> MemberName {
        match name_node.kind() {
            kind::PROPERTY_IDENTIFIER | kind::IDENTIFIER => {
                MemberName::Identifier(self.text(name_node))
            }
            kind::PRIVATE_PROPERTY_IDENTIFIER => MemberName::Private(self.text(name_node)),
            kind::STRING => MemberName::StringKey(key_text(name_node, self.src)),
            kind::NUMBER => MemberName::Numeric(self.text(name_node)),
            kind::COMPUTED_PROPERTY_NAME => {
                let inner = name_node.named_child(0);
                match inner.and_then(|n| member_chain(n, self.src)) {
                    Some(parts) if parts.len() == 2 && parts[0] == "Symbol" => {
                        MemberName::WellKnownSymbol(parts[1].clone())
                    }
                    _ => MemberName::Computed,
                }
            }
            _ => MemberName::Identifier(self.text(name_node)),
        }
    }

    fn visibility_of(&self, node: Node<'t>, member: &MemberName) -> Visibility {
        if matches!(member, MemberName::Private(_)) {
            return Visibility::EcmaPrivate;
        }
        self.accessibility(node).unwrap_or(Visibility::Public)
    }

    fn visit_class_body(&mut self, body: Node<'t>, scope: &Scope) {
        if scope.depth >= MAX_VISIT_DEPTH {
            depth_limit(self.sink, self.range(body));
            return;
        }
        let children: Vec<Node<'t>> = {
            let mut cursor = body.walk();
            body.named_children(&mut cursor).collect()
        };
        let mut pending: Vec<Node<'t>> = Vec::new();
        let mut sigs: Vec<(String, bool, Node<'t>)> = Vec::new();
        for child in children {
            match child.kind() {
                kind::DECORATOR => pending.push(child),
                kind::COMMENT => {}
                kind::METHOD_SIGNATURE => {
                    let name = child
                        .child_by_field_name(field::NAME)
                        .map(|n| self.text(n))
                        .unwrap_or_default();
                    let is_static = self.has_token(child, "static");
                    if sigs
                        .last()
                        .is_some_and(|(n, s, _)| *n != name || *s != is_static)
                    {
                        self.flush_signatures(&mut sigs, scope, &mut pending);
                    }
                    sigs.push((name, is_static, child));
                }
                kind::METHOD_DEFINITION => {
                    let name_text = child
                        .child_by_field_name(field::NAME)
                        .map(|n| self.text(n))
                        .unwrap_or_default();
                    let is_static = self.has_token(child, "static");
                    let overloads: Vec<String> = if sigs
                        .last()
                        .is_some_and(|(n, s, _)| *n == name_text && *s == is_static)
                    {
                        sigs.iter()
                            .map(|(n, _, node)| self.signature_of_node(*node, n))
                            .collect()
                    } else {
                        self.flush_signatures(&mut sigs, scope, &mut pending);
                        Vec::new()
                    };
                    sigs.clear();
                    let decorators = std::mem::take(&mut pending);
                    self.visit_method(child, scope, decorators, overloads, false);
                }
                kind::ABSTRACT_METHOD_SIGNATURE => {
                    self.flush_signatures(&mut sigs, scope, &mut pending);
                    let decorators = std::mem::take(&mut pending);
                    self.visit_method(child, scope, decorators, Vec::new(), true);
                }
                kind::PUBLIC_FIELD_DEFINITION => {
                    self.flush_signatures(&mut sigs, scope, &mut pending);
                    let decorators = std::mem::take(&mut pending);
                    self.visit_field(child, scope, decorators);
                }
                _ => {
                    self.flush_signatures(&mut sigs, scope, &mut pending);
                    pending.clear();
                }
            }
        }
        self.flush_signatures(&mut sigs, scope, &mut pending);
    }

    /// Method signatures without an implementation (ambient or abstract-like classes): the
    /// first signature is the symbol, the rest are its overloads.
    fn flush_signatures(
        &mut self,
        sigs: &mut Vec<(String, bool, Node<'t>)>,
        scope: &Scope,
        pending: &mut Vec<Node<'t>>,
    ) {
        if sigs.is_empty() {
            return;
        }
        let taken = std::mem::take(sigs);
        let first = taken[0].2;
        let overloads: Vec<String> = taken
            .iter()
            .skip(1)
            .map(|(n, _, node)| self.signature_of_node(*node, n))
            .collect();
        let decorators = std::mem::take(pending);
        self.visit_method(first, scope, decorators, overloads, false);
    }

    fn decorators_with_children(
        &self,
        node: Node<'t>,
        preceding: Vec<Node<'t>>,
    ) -> (Vec<IrDecorator>, Option<Node<'t>>) {
        let mut all: Vec<(IrDecorator, Node<'t>)> = preceding
            .into_iter()
            .filter_map(|d| self.decorator(d).map(|ir| (ir, d)))
            .collect();
        all.extend(self.decorators_of(node));
        let start = all.first().map(|(_, n)| *n);
        (all.into_iter().map(|(d, _)| d).collect(), start)
    }

    fn visit_method(
        &mut self,
        node: Node<'t>,
        scope: &Scope,
        preceding: Vec<Node<'t>>,
        overloads: Vec<String>,
        abstract_sig: bool,
    ) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let member = self.member_name(name_node);
        let (decorators, deco_start) = self.decorators_with_children(node, preceding);
        let mut kind = SymbolKind::Method;
        if self.has_token(node, "get") {
            kind = SymbolKind::Getter;
        } else if self.has_token(node, "set") {
            kind = SymbolKind::Setter;
        }
        if matches!(&member, MemberName::Identifier(n) if n == "constructor")
            && !self.has_token(node, "static")
        {
            kind = SymbolKind::Constructor;
        }
        let visibility = self.visibility_of(node, &member);
        let decision = qualify(scope.qn.as_ref(), Construct::Member(member));
        let range = match deco_start {
            Some(start) if start.start_byte() < node.start_byte() => {
                self.range_between(start, node)
            }
            _ => self.range(node),
        };
        let body = node.child_by_field_name(field::BODY);
        let (params, params_node) = self.params(node);
        let nodes = SymNodes {
            node,
            body,
            params: params_node,
        };
        let Some(id) = self.add(scope, kind, decision, range, nodes, node) else {
            return;
        };
        let mut modifiers = self.member_modifiers(node);
        if abstract_sig {
            modifiers.insert(Modifiers::ABSTRACT);
        }
        let tparams = self.type_params(node);
        let ret = node
            .child_by_field_name(field::RETURN_TYPE)
            .and_then(|r| self.type_text(r));
        let name = self.sym(id).name.clone();
        let signature = self.signature(modifiers, &name, &tparams, &params, ret.as_deref());
        {
            let body_range = body.map(|b| self.range(b));
            let name_range = Some(self.range(name_node));
            let sym = self.sym(id);
            sym.modifiers.insert(modifiers);
            sym.visibility = visibility;
            sym.decorators = decorators;
            sym.type_params = tparams;
            sym.params = params.clone();
            sym.return_type = ret;
            sym.signature = Some(signature);
            sym.overload_signatures = overloads;
            sym.body_range = body_range;
            sym.name_range = name_range;
        }
        if kind == SymbolKind::Constructor {
            self.constructor_properties(node, scope, &params);
        }
    }

    /// Constructor parameter properties are synthetic property symbols of the class.
    fn constructor_properties(&mut self, ctor: Node<'t>, scope: &Scope, params: &[IrParam]) {
        let Some(list) = ctor.child_by_field_name(field::PARAMETERS) else {
            return;
        };
        let mut cursor = list.walk();
        let nodes: Vec<Node<'t>> = list
            .named_children(&mut cursor)
            .filter(|p| {
                matches!(
                    p.kind(),
                    kind::REQUIRED_PARAMETER | kind::OPTIONAL_PARAMETER
                )
            })
            .collect();
        for (param_node, param) in nodes.into_iter().zip(params) {
            let Some(property) = param.property else {
                continue;
            };
            let decision = qualify(
                scope.qn.as_ref(),
                Construct::Member(MemberName::Identifier(param.name.clone())),
            );
            let range = self.range(param_node);
            let sn = SymNodes {
                node: param_node,
                body: None,
                params: None,
            };
            let Some(id) = self.add(scope, SymbolKind::Property, decision, range, sn, param_node)
            else {
                continue;
            };
            let sym = self.sym(id);
            sym.visibility = property.visibility;
            if property.readonly {
                sym.modifiers.insert(Modifiers::READONLY);
            }
            if param.optional {
                sym.modifiers.insert(Modifiers::OPTIONAL);
            }
            sym.declared_type = param.type_text.clone();
            sym.attrs
                .insert("from_constructor_param".to_owned(), AttrValue::Bool(true));
            sym.name_range = Some(range);
        }
    }

    fn visit_field(&mut self, node: Node<'t>, scope: &Scope, preceding: Vec<Node<'t>>) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let member = self.member_name(name_node);
        let (decorators, deco_start) = self.decorators_with_children(node, preceding);
        let value = node.child_by_field_name(field::VALUE);
        let arrow_like = value.filter(|v| {
            matches!(
                v.kind(),
                kind::ARROW_FUNCTION | kind::FUNCTION_EXPRESSION | kind::GENERATOR_FUNCTION
            )
        });
        let visibility = self.visibility_of(node, &member);
        let decision = qualify(scope.qn.as_ref(), Construct::Member(member));
        let range = match deco_start {
            Some(start) if start.start_byte() < node.start_byte() => {
                self.range_between(start, node)
            }
            _ => self.range(node),
        };
        let kind = if arrow_like.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Property
        };
        let nodes = SymNodes {
            node,
            body: arrow_like
                .and_then(|f| f.child_by_field_name(field::BODY))
                .or(value),
            params: None,
        };
        let Some(id) = self.add(scope, kind, decision, range, nodes, node) else {
            return;
        };
        let mut modifiers = self.member_modifiers(node);
        let declared = node
            .child_by_field_name(field::TYPE)
            .and_then(|t| self.type_text(t));
        let name_range = Some(self.range(name_node));
        let value_range = value.map(|v| self.range(v));
        let (params, ret, tparams) = match arrow_like {
            Some(f) => {
                if self.has_token(f, "async") {
                    modifiers.insert(Modifiers::ASYNC);
                }
                let (params, _) = self.params(f);
                let ret = f
                    .child_by_field_name(field::RETURN_TYPE)
                    .and_then(|r| self.type_text(r));
                (params, ret, self.type_params(f))
            }
            None => (Vec::new(), None, Vec::new()),
        };
        let name = self.sym(id).name.clone();
        let signature =
            arrow_like.map(|_| self.signature(modifiers, &name, &tparams, &params, ret.as_deref()));
        let const_value = if arrow_like.is_none() {
            value.and_then(|v| self.const_value(v))
        } else {
            None
        };
        let redacted = const_value.is_some() && is_secret_name(&name);
        let sym = self.sym(id);
        sym.modifiers.insert(modifiers);
        sym.visibility = visibility;
        sym.decorators = decorators;
        sym.declared_type = declared;
        sym.name_range = name_range;
        sym.body_range = value_range;
        sym.signature = signature;
        sym.params = params;
        sym.return_type = ret;
        sym.type_params = tparams;
        if arrow_like.is_some() {
            sym.attrs.insert(
                "binding".to_owned(),
                AttrValue::Str("arrow_property".to_owned()),
            );
        }
        if redacted {
            sym.attrs
                .insert("redacted".to_owned(), AttrValue::Bool(true));
        } else {
            sym.const_value = const_value;
        }
    }

    // ----- interfaces, aliases, enums ----------------------------------------------------

    fn visit_interface(&mut self, node: Node<'t>, scope: &Scope, ctx: StmtCtx<'t>) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let wrapper = ctx.wrapper.unwrap_or(node);
        let range = self.range(wrapper);
        let decision = qualify(scope.qn.as_ref(), Construct::Declared(self.text(name_node)));
        let body = node.child_by_field_name(field::BODY);
        let nodes = SymNodes {
            node: wrapper,
            body,
            params: None,
        };
        let Some(id) = self.add(scope, SymbolKind::Interface, decision, range, nodes, node) else {
            return;
        };
        self.table.by_node.insert(wrapper.id(), id);
        let mut extends = Vec::new();
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == kind::EXTENDS_TYPE_CLAUSE {
                    let mut c2 = child.walk();
                    for part in child.named_children(&mut c2) {
                        if part.kind() != kind::COMMENT {
                            extends.push(truncate_chars(&collapse_ws(&self.text(part)), 200));
                        }
                    }
                }
            }
        }
        let tparams = self.type_params(node);
        let name_range = Some(self.range(name_node));
        let body_range = body.map(|b| self.range(b));
        let sym = self.sym(id);
        if ctx.exported {
            sym.modifiers.insert(Modifiers::EXPORTED);
        }
        if ctx.default {
            sym.modifiers.insert(Modifiers::DEFAULT_EXPORT);
        }
        if ctx.ambient {
            sym.modifiers.insert(Modifiers::DECLARE);
        }
        sym.heritage.extends = extends;
        sym.type_params = tparams;
        sym.name_range = name_range;
        sym.body_range = body_range;
        let name = sym.name.clone();
        sym.signature = Some(format!("interface {name}"));
        if let Some(body) = body {
            let inner = self.child_scope(id, scope, false);
            self.visit_interface_body(body, &inner);
        }
    }

    fn visit_interface_body(&mut self, body: Node<'t>, scope: &Scope) {
        let children: Vec<Node<'t>> = {
            let mut cursor = body.walk();
            body.named_children(&mut cursor)
                .filter(|c| c.kind() != kind::COMMENT)
                .collect()
        };
        let mut i = 0;
        while i < children.len() {
            let child = children[i];
            match child.kind() {
                kind::PROPERTY_SIGNATURE => {
                    self.visit_property_signature(child, scope);
                    i += 1;
                }
                kind::METHOD_SIGNATURE => {
                    let name = child
                        .child_by_field_name(field::NAME)
                        .map(|n| self.text(n))
                        .unwrap_or_default();
                    let mut j = i + 1;
                    while j < children.len()
                        && children[j].kind() == kind::METHOD_SIGNATURE
                        && children[j]
                            .child_by_field_name(field::NAME)
                            .map(|n| self.text(n))
                            .as_deref()
                            == Some(name.as_str())
                    {
                        j += 1;
                    }
                    let overloads: Vec<String> = children[i + 1..j]
                        .iter()
                        .map(|s| self.signature_of_node(*s, &name))
                        .collect();
                    self.visit_method(child, scope, Vec::new(), overloads, false);
                    i = j;
                }
                _ => i += 1,
            }
        }
    }

    fn visit_property_signature(&mut self, node: Node<'t>, scope: &Scope) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let member = self.member_name(name_node);
        let decision = qualify(scope.qn.as_ref(), Construct::Member(member));
        let range = self.range(node);
        let nodes = SymNodes {
            node,
            body: None,
            params: None,
        };
        let Some(id) = self.add(scope, SymbolKind::Property, decision, range, nodes, node) else {
            return;
        };
        let modifiers = self.member_modifiers(node);
        let declared = node
            .child_by_field_name(field::TYPE)
            .and_then(|t| self.type_text(t));
        let name_range = Some(self.range(name_node));
        let sym = self.sym(id);
        sym.modifiers.insert(modifiers);
        sym.declared_type = declared;
        sym.name_range = name_range;
    }

    fn visit_type_alias(&mut self, node: Node<'t>, scope: &Scope, ctx: StmtCtx<'t>) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let wrapper = ctx.wrapper.unwrap_or(node);
        let range = self.range(wrapper);
        let decision = qualify(scope.qn.as_ref(), Construct::Declared(self.text(name_node)));
        let value = node.child_by_field_name(field::VALUE);
        let nodes = SymNodes {
            node: wrapper,
            body: value,
            params: None,
        };
        let Some(id) = self.add(scope, SymbolKind::TypeAlias, decision, range, nodes, node) else {
            return;
        };
        self.table.by_node.insert(wrapper.id(), id);
        let tparams = self.type_params(node);
        let name_range = Some(self.range(name_node));
        let body_range = value.map(|v| self.range(v));
        let sym = self.sym(id);
        if ctx.exported {
            sym.modifiers.insert(Modifiers::EXPORTED);
        }
        if ctx.default {
            sym.modifiers.insert(Modifiers::DEFAULT_EXPORT);
        }
        if ctx.ambient {
            sym.modifiers.insert(Modifiers::DECLARE);
        }
        sym.type_params = tparams;
        sym.name_range = name_range;
        sym.body_range = body_range;
        let name = sym.name.clone();
        sym.signature = Some(format!("type {name}"));
    }

    fn visit_enum(&mut self, node: Node<'t>, scope: &Scope, ctx: StmtCtx<'t>) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let wrapper = ctx.wrapper.unwrap_or(node);
        let range = self.range(wrapper);
        let decision = qualify(scope.qn.as_ref(), Construct::Declared(self.text(name_node)));
        let body = node.child_by_field_name(field::BODY);
        let nodes = SymNodes {
            node: wrapper,
            body,
            params: None,
        };
        let Some(id) = self.add(scope, SymbolKind::Enum, decision, range, nodes, node) else {
            return;
        };
        self.table.by_node.insert(wrapper.id(), id);
        let is_const = self.has_token(node, "const");
        let name_range = Some(self.range(name_node));
        let body_range = body.map(|b| self.range(b));
        {
            let sym = self.sym(id);
            if ctx.exported {
                sym.modifiers.insert(Modifiers::EXPORTED);
            }
            if ctx.default {
                sym.modifiers.insert(Modifiers::DEFAULT_EXPORT);
            }
            if ctx.ambient {
                sym.modifiers.insert(Modifiers::DECLARE);
            }
            if is_const {
                sym.modifiers.insert(Modifiers::CONST_ENUM);
            }
            sym.name_range = name_range;
            sym.body_range = body_range;
            let name = sym.name.clone();
            sym.signature = Some(if is_const {
                format!("const enum {name}")
            } else {
                format!("enum {name}")
            });
        }
        let Some(body) = body else { return };
        let inner = self.child_scope(id, scope, false);
        let members: Vec<Node<'t>> = {
            let mut cursor = body.walk();
            body.named_children(&mut cursor)
                .filter(|c| c.kind() != kind::COMMENT)
                .collect()
        };
        for member in members {
            let (name_node, value) = match member.kind() {
                kind::ENUM_ASSIGNMENT => (
                    member.child_by_field_name(field::NAME),
                    member.child_by_field_name(field::VALUE),
                ),
                _ => (Some(member), None),
            };
            let Some(name_node) = name_node else { continue };
            let member_name = self.member_name(name_node);
            let decision = qualify(inner.qn.as_ref(), Construct::Member(member_name));
            let mrange = self.range(member);
            let nodes = SymNodes {
                node: member,
                body: value,
                params: None,
            };
            let Some(mid) = self.add(
                &inner,
                SymbolKind::EnumMember,
                decision,
                mrange,
                nodes,
                member,
            ) else {
                continue;
            };
            let const_value = value.and_then(|v| self.const_value(v));
            let name_range = Some(self.range(name_node));
            let body_range = value.map(|v| self.range(v));
            let sym = self.sym(mid);
            sym.const_value = const_value;
            sym.name_range = name_range;
            sym.body_range = body_range;
        }
    }

    // ----- functions -------------------------------------------------------------------

    fn visit_function(&mut self, node: Node<'t>, scope: &Scope, ctx: StmtCtx<'t>) {
        let Some(name_node) = node.child_by_field_name(field::NAME) else {
            return;
        };
        let wrapper = ctx.wrapper.unwrap_or(node);
        let range = self.range(wrapper);
        let decision = qualify(scope.qn.as_ref(), Construct::Declared(self.text(name_node)));
        let body = node.child_by_field_name(field::BODY);
        let (params, params_node) = self.params(node);
        let nodes = SymNodes {
            node: wrapper,
            body,
            params: params_node,
        };
        let Some(id) = self.add(scope, SymbolKind::Function, decision, range, nodes, node) else {
            return;
        };
        self.table.by_node.insert(wrapper.id(), id);
        let name = self.sym(id).name.clone();
        let _ = params;
        self.fill_function(id, node, &ctx, &name);
        self.sym(id).name_range = Some(self.range(name_node));
        if self.cfg.anonymous_functions == AnonymousFnPolicy::Emit {
            // Anonymous functions inside this body are emitted by `emit_anonymous`.
        }
    }

    /// Fills parameters, types, modifiers and the signature of a function-like node.
    fn fill_function(&mut self, id: LocalId, node: Node<'t>, ctx: &StmtCtx<'t>, name: &str) {
        let mut modifiers = self.member_modifiers(node);
        if node.kind() == kind::GENERATOR_FUNCTION_DECLARATION
            || node.kind() == kind::GENERATOR_FUNCTION
        {
            modifiers.insert(Modifiers::GENERATOR);
        }
        if ctx.exported {
            modifiers.insert(Modifiers::EXPORTED);
        }
        if ctx.default {
            modifiers.insert(Modifiers::DEFAULT_EXPORT);
        }
        if ctx.ambient {
            modifiers.insert(Modifiers::DECLARE);
        }
        let tparams = self.type_params(node);
        let (params, _) = self.params(node);
        let ret = node
            .child_by_field_name(field::RETURN_TYPE)
            .and_then(|r| self.type_text(r));
        let signature = self.signature(modifiers, name, &tparams, &params, ret.as_deref());
        let body_range = node.child_by_field_name(field::BODY).map(|b| self.range(b));
        let sym = self.sym(id);
        sym.modifiers.insert(modifiers);
        sym.type_params = tparams;
        sym.params = params;
        sym.return_type = ret;
        sym.signature = Some(signature);
        sym.overload_signatures = ctx.overloads.clone();
        sym.body_range = body_range;
    }

    // ----- variables -------------------------------------------------------------------

    fn const_value(&self, value: Node<'t>) -> Option<ConstValue> {
        let node = if value.kind() == kind::AS_EXPRESSION {
            value.named_child(0)?
        } else {
            value
        };
        match node.kind() {
            kind::STRING => {
                let s = string_content(node, self.src);
                Some(ConstValue::Str(cap_bytes(s, MAX_CONST_STRING_BYTES)))
            }
            kind::NUMBER => Some(ConstValue::Num(
                self.text(node).replace('_', "").to_lowercase(),
            )),
            kind::TRUE => Some(ConstValue::Bool(true)),
            kind::FALSE => Some(ConstValue::Bool(false)),
            kind::TEMPLATE_STRING => {
                let mut cursor = node.walk();
                let has_subst = node
                    .named_children(&mut cursor)
                    .any(|c| c.kind() == kind::TEMPLATE_SUBSTITUTION);
                if has_subst {
                    None
                } else {
                    let raw = self.text(node);
                    Some(ConstValue::Str(cap_bytes(
                        raw.trim_matches('`').to_owned(),
                        MAX_CONST_STRING_BYTES,
                    )))
                }
            }
            _ => None,
        }
    }

    fn visit_variables(&mut self, node: Node<'t>, scope: &Scope, ctx: StmtCtx<'t>) {
        let is_const = node
            .child_by_field_name(field::KIND)
            .map(|k| self.text(k) == "const")
            .unwrap_or(false);
        let wrapper = ctx.wrapper.unwrap_or(node);
        let declarators: Vec<Node<'t>> = {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .filter(|c| c.kind() == kind::VARIABLE_DECLARATOR)
                .collect()
        };
        for declarator in declarators {
            let Some(name_node) = declarator.child_by_field_name(field::NAME) else {
                continue;
            };
            let value = declarator.child_by_field_name(field::VALUE);
            let declared_type = declarator
                .child_by_field_name(field::TYPE)
                .and_then(|t| self.type_text(t));
            // The symbol's range is the whole statement for a single declarator, else the
            // declarator itself.
            let range = if node.named_child_count() <= 2 {
                self.range(wrapper)
            } else {
                self.range(declarator)
            };
            let names: Vec<(Node<'t>, bool)> = match name_node.kind() {
                kind::IDENTIFIER => vec![(name_node, false)],
                kind::OBJECT_PATTERN | kind::ARRAY_PATTERN => {
                    let mut leaves = Vec::new();
                    self.pattern_leaves(name_node, &mut leaves, 0);
                    leaves.into_iter().map(|n| (n, true)).collect()
                }
                _ => Vec::new(),
            };
            for (leaf, destructured) in names {
                self.visit_variable(
                    wrapper,
                    declarator,
                    leaf,
                    value,
                    declared_type.clone(),
                    destructured,
                    is_const,
                    range,
                    scope,
                    &ctx,
                );
            }
        }
    }

    fn pattern_leaves(&self, pattern: Node<'t>, out: &mut Vec<Node<'t>>, depth: usize) {
        if depth > 16 {
            return;
        }
        let mut cursor = pattern.walk();
        for child in pattern.named_children(&mut cursor) {
            match child.kind() {
                kind::IDENTIFIER | kind::SHORTHAND_PROPERTY_IDENTIFIER_PATTERN => out.push(child),
                kind::PAIR_PATTERN => {
                    if let Some(v) = child.child_by_field_name(field::VALUE) {
                        self.leaf_or_nested(v, out, depth);
                    }
                }
                kind::REST_PATTERN => {
                    if let Some(v) = child.named_child(0) {
                        self.leaf_or_nested(v, out, depth);
                    }
                }
                kind::OBJECT_ASSIGNMENT_PATTERN | kind::ASSIGNMENT_PATTERN => {
                    if let Some(left) = child.child_by_field_name(field::LEFT) {
                        self.leaf_or_nested(left, out, depth);
                    }
                }
                kind::OBJECT_PATTERN | kind::ARRAY_PATTERN => {
                    self.pattern_leaves(child, out, depth + 1)
                }
                _ => {}
            }
        }
    }

    fn leaf_or_nested(&self, node: Node<'t>, out: &mut Vec<Node<'t>>, depth: usize) {
        match node.kind() {
            kind::IDENTIFIER | kind::SHORTHAND_PROPERTY_IDENTIFIER_PATTERN => out.push(node),
            kind::OBJECT_PATTERN | kind::ARRAY_PATTERN => self.pattern_leaves(node, out, depth + 1),
            kind::ASSIGNMENT_PATTERN | kind::OBJECT_ASSIGNMENT_PATTERN => {
                if let Some(left) = node.child_by_field_name(field::LEFT) {
                    self.leaf_or_nested(left, out, depth + 1);
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_variable(
        &mut self,
        wrapper: Node<'t>,
        declarator: Node<'t>,
        name_node: Node<'t>,
        value: Option<Node<'t>>,
        declared_type: Option<String>,
        destructured: bool,
        is_const: bool,
        range: SourceRange,
        scope: &Scope,
        ctx: &StmtCtx<'t>,
    ) {
        let name = self.text(name_node);
        let decision = qualify(scope.qn.as_ref(), Construct::Declared(name.clone()));
        let function_value = value.filter(|v| {
            !destructured
                && matches!(
                    v.kind(),
                    kind::ARROW_FUNCTION | kind::FUNCTION_EXPRESSION | kind::GENERATOR_FUNCTION
                )
        });
        let class_value = value.filter(|v| !destructured && v.kind() == kind::CLASS);
        let object_value = value.filter(|v| !destructured && is_const && v.kind() == kind::OBJECT);
        let kind_of = if function_value.is_some() {
            SymbolKind::Function
        } else if class_value.is_some() {
            SymbolKind::Class
        } else if is_const {
            SymbolKind::Constant
        } else {
            SymbolKind::Variable
        };
        let body = function_value
            .and_then(|f| f.child_by_field_name(field::BODY))
            .or(class_value.and_then(|c| c.child_by_field_name(field::BODY)))
            .or(value);
        let nodes = SymNodes {
            node: wrapper,
            body,
            params: None,
        };
        let Some(id) = self.add(scope, kind_of, decision, range, nodes, declarator) else {
            return;
        };
        if !destructured {
            self.table.by_node.entry(wrapper.id()).or_insert(id);
        }
        let name_range = Some(self.range(name_node));
        let value_range = value.map(|v| self.range(v));
        let const_value = if function_value.is_some() || class_value.is_some() || destructured {
            None
        } else {
            value.and_then(|v| self.const_value(v))
        };
        let redacted = const_value.is_some() && is_secret_name(&name);
        {
            let sym = self.sym(id);
            if ctx.exported {
                sym.modifiers.insert(Modifiers::EXPORTED);
            }
            if ctx.default {
                sym.modifiers.insert(Modifiers::DEFAULT_EXPORT);
            }
            if ctx.ambient {
                sym.modifiers.insert(Modifiers::DECLARE);
            }
            sym.declared_type = declared_type;
            sym.name_range = name_range;
            sym.body_range = value_range;
            if destructured {
                sym.attrs
                    .insert("destructured".to_owned(), AttrValue::Bool(true));
            }
            if redacted {
                sym.attrs
                    .insert("redacted".to_owned(), AttrValue::Bool(true));
            } else {
                sym.const_value = const_value;
            }
        }
        if let Some(function) = function_value {
            let binding = if function.kind() == kind::ARROW_FUNCTION {
                "const_arrow"
            } else {
                "const_function_expr"
            };
            self.fill_function(
                id,
                function,
                &StmtCtx {
                    exported: false,
                    default: false,
                    ambient: false,
                    ..ctx.clone()
                },
                &name,
            );
            self.sym(id)
                .attrs
                .insert("binding".to_owned(), AttrValue::Str(binding.to_owned()));
            let (_, params_node) = self.params(function);
            self.table.nodes[id.0 as usize].params = params_node;
            if let Some(vr) = value_range {
                self.sym(id).body_range = function
                    .child_by_field_name(field::BODY)
                    .map(|b| self.range(b))
                    .or(Some(vr));
            }
        } else if let Some(class) = class_value {
            self.sym(id).attrs.insert(
                "binding".to_owned(),
                AttrValue::Str("const_class".to_owned()),
            );
            let (extends, implements) = self.heritage(class);
            let sym = self.sym(id);
            sym.heritage.extends = extends;
            sym.heritage.implements = implements;
            if let Some(body) = class.child_by_field_name(field::BODY) {
                let inner = self.child_scope(id, scope, false);
                self.visit_class_body(body, &inner);
            }
        } else if let Some(object) = object_value {
            let inner = self.child_scope(id, scope, false);
            self.visit_object(object, &inner, 1);
        }
    }

    // ----- object literals -------------------------------------------------------------

    fn visit_object(&mut self, object: Node<'t>, scope: &Scope, depth: usize) {
        let members: Vec<Node<'t>> = {
            let mut cursor = object.walk();
            object
                .named_children(&mut cursor)
                .filter(|c| c.kind() != kind::COMMENT)
                .collect()
        };
        for member in members {
            match member.kind() {
                kind::METHOD_DEFINITION => {
                    self.visit_method(member, scope, Vec::new(), Vec::new(), false);
                }
                kind::PAIR => {
                    let Some(key) = member.child_by_field_name(field::KEY) else {
                        continue;
                    };
                    let Some(value) = member.child_by_field_name(field::VALUE) else {
                        continue;
                    };
                    let member_name = self.member_name(key);
                    match value.kind() {
                        kind::ARROW_FUNCTION
                        | kind::FUNCTION_EXPRESSION
                        | kind::GENERATOR_FUNCTION => {
                            let decision =
                                qualify(scope.qn.as_ref(), Construct::Member(member_name));
                            let range = self.range(member);
                            let nodes = SymNodes {
                                node: member,
                                body: value.child_by_field_name(field::BODY),
                                params: None,
                            };
                            let Some(id) = self.add(
                                scope,
                                SymbolKind::Function,
                                decision,
                                range,
                                nodes,
                                member,
                            ) else {
                                continue;
                            };
                            let name = self.sym(id).name.clone();
                            self.fill_function(id, value, &StmtCtx::default(), &name);
                            let key_range = self.range(key);
                            let sym = self.sym(id);
                            sym.attrs.insert(
                                "binding".to_owned(),
                                AttrValue::Str("object_pair_fn".to_owned()),
                            );
                            sym.name_range = Some(key_range);
                            let (_, params_node) = self.params(value);
                            self.table.nodes[id.0 as usize].params = params_node;
                        }
                        kind::OBJECT if depth < 2 => {
                            // Nested object: members are qualified below it, the object itself
                            // is not a symbol.
                            if let NameDecision::Named(qn) =
                                qualify(scope.qn.as_ref(), Construct::Member(member_name))
                            {
                                let nested = Scope {
                                    parent: scope.parent,
                                    qn: Some(qn),
                                    ambient: scope.ambient,
                                    depth: scope.depth + 1,
                                };
                                self.visit_object(value, &nested, depth + 1);
                            }
                        }
                        _ => {
                            if matches!(member_name, MemberName::Computed) {
                                self.sink.note(
                                    DiagSeverity::Info,
                                    DiagCode::ComputedMemberName,
                                    "computed object key skipped",
                                    Some(self.range(key)),
                                );
                            }
                        }
                    }
                    if key.kind() == kind::COMPUTED_PROPERTY_NAME
                        && matches!(self.member_name(key), MemberName::Computed)
                        && matches!(
                            value.kind(),
                            kind::ARROW_FUNCTION
                                | kind::FUNCTION_EXPRESSION
                                | kind::GENERATOR_FUNCTION
                        )
                    {
                        // Already reported through `qualify` -> Skip in `add`.
                    }
                }
                _ => {}
            }
        }
    }

    // ----- anonymous functions (Emit policy) -----------------------------------------------

    fn emit_anonymous(&mut self) {
        // Snapshot the work first: emitting appends symbols while we iterate.
        let work: Vec<(LocalId, Node<'t>)> = self
            .table
            .nodes
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(i, n)| n.body.map(|b| (LocalId(i as u32), b)))
            .filter(|(id, _)| {
                matches!(
                    self.table.symbols[id.0 as usize].kind,
                    SymbolKind::Function
                        | SymbolKind::Method
                        | SymbolKind::Getter
                        | SymbolKind::Setter
                        | SymbolKind::Constructor
                )
            })
            .collect();
        for (owner, body) in work {
            let mut stack: Vec<(Node<'t>, usize)> = vec![(body, 0)];
            while let Some((node, depth)) = stack.pop() {
                if depth > MAX_VISIT_DEPTH {
                    depth_limit(self.sink, self.range(node));
                    continue;
                }
                let is_fn = matches!(
                    node.kind(),
                    kind::ARROW_FUNCTION | kind::FUNCTION_EXPRESSION | kind::GENERATOR_FUNCTION
                );
                if is_fn && !self.table.by_node.contains_key(&node.id()) {
                    let owner_qn = self.table.symbols[owner.0 as usize].qualified_name.clone();
                    let decision = qualify(Some(&owner_qn), Construct::Anonymous);
                    let scope = Scope {
                        parent: owner,
                        qn: Some(owner_qn),
                        ambient: false,
                        depth,
                    };
                    let range = self.range(node);
                    let nodes = SymNodes {
                        node,
                        body: node.child_by_field_name(field::BODY),
                        params: None,
                    };
                    if let Some(id) =
                        self.add(&scope, SymbolKind::Function, decision, range, nodes, node)
                    {
                        self.fill_function(id, node, &StmtCtx::default(), "<anonymous>");
                        self.sym(id)
                            .attrs
                            .insert("binding".to_owned(), AttrValue::Str("anonymous".to_owned()));
                    }
                }
                let mut cursor = node.walk();
                let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
                for child in children.into_iter().rev() {
                    stack.push((child, depth + 1));
                }
            }
        }
    }
}

fn cap_bytes(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s
}

/// Marks symbols whose range overlaps an error range.
pub fn mark_errors(symbols: &mut [IrSymbol], error_ranges: &[SourceRange]) {
    if error_ranges.is_empty() {
        return;
    }
    let overlaps = |a: &SourceRange, b: &SourceRange| -> bool {
        let (a_start, a_end): (Position, Position) = (a.start, a.end);
        a_start <= b.end && b.start <= a_end
    };
    for symbol in symbols {
        if error_ranges.iter().any(|e| overlaps(&symbol.range, e)) {
            symbol.has_errors = true;
        }
    }
}
