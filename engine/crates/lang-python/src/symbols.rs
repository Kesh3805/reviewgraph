//! Declarations: modules, classes, functions, methods, properties, class attributes and module
//! level variables and constants. Nested functions and classes inside function bodies are not
//! symbols; their facts and references fold into the enclosing symbol.

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::{
    AttrValue, ConstValue, DiagCode, DiagSeverity, IrDecorator, IrExpr, IrParam, IrSymbol, LocalId,
    Modifiers, Visibility,
};
use review_core::symbol::{Hash128, SymbolKind};
use tree_sitter::Node;

use crate::diagnostics::DiagnosticSink;
use crate::exprs::{call_args, children_of, collapse_ws, dotted, expr, string_value};
use crate::text::Source;

/// Deepest nesting the declaration walk descends into.
pub const MAX_VISIT_DEPTH: usize = 256;

/// Longest signature text kept on a symbol.
const MAX_SIGNATURE_CHARS: usize = 512;

/// Name fragments whose constant values are never stored.
const SECRET_FRAGMENTS: &[&str] = &[
    "secret",
    "token",
    "password",
    "api_key",
    "apikey",
    "private_key",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Module,
    Class,
}

/// The syntax nodes behind one symbol.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'t> {
    /// The whole declaration including decorators (`decorated_definition` when present).
    pub outer: Node<'t>,
    /// The node whose tokens form the body hash.
    pub body: Option<Node<'t>>,
}

/// Every symbol of a file plus the nodes they came from. `entries[i]` belongs to `symbols[i]`.
#[derive(Debug)]
pub struct Table<'t> {
    pub symbols: Vec<IrSymbol>,
    pub entries: Vec<Entry<'t>>,
}

struct Collector<'a, 't> {
    src: Source<'a>,
    sink: &'a mut DiagnosticSink,
    table: Table<'t>,
    exported: Option<BTreeSet<String>>,
    seen_vars: BTreeSet<(u32, String, &'static str)>,
}

/// Collects the declarations of `root`. `module` is the already built module symbol.
pub fn collect<'a, 't>(
    root: Node<'t>,
    src: Source<'a>,
    sink: &'a mut DiagnosticSink,
    mut module: IrSymbol,
) -> Table<'t> {
    module.doc_hash = docstring_hash(root, &src);
    let exported = module_all(root, &src);
    let mut collector = Collector {
        src,
        sink,
        table: Table {
            symbols: vec![module],
            entries: vec![Entry {
                outer: root,
                body: Some(root),
            }],
        },
        exported,
        seen_vars: BTreeSet::new(),
    };
    collector.visit_scope(root, LocalId(0), Scope::Module, 0);
    collector.table
}

/// Names listed in a module level `__all__`, when there is one.
fn module_all(root: Node<'_>, src: &Source<'_>) -> Option<BTreeSet<String>> {
    let mut names: Option<BTreeSet<String>> = None;
    for stmt in children_of(root) {
        if stmt.kind() != "expression_statement" {
            continue;
        }
        for child in children_of(stmt) {
            if child.kind() != "assignment" && child.kind() != "augmented_assignment" {
                continue;
            }
            let (Some(left), Some(right)) = (
                child.child_by_field_name("left"),
                child.child_by_field_name("right"),
            ) else {
                continue;
            };
            if left.kind() != "identifier" || src.text(left) != "__all__" {
                continue;
            }
            if right.kind() != "list" && right.kind() != "tuple" {
                continue;
            }
            let set = names.get_or_insert_with(BTreeSet::new);
            for item in children_of(right) {
                if let Some(value) = string_value(item, src) {
                    set.insert(value);
                }
            }
        }
    }
    names
}

/// Hash of the docstring of a module or block, when its first statement is a string.
fn docstring_hash(scope: Node<'_>, src: &Source<'_>) -> Option<Hash128> {
    let first = children_of(scope).into_iter().next()?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let string = children_of(first).into_iter().next()?;
    let text = string_value(string, src)?;
    Some(Hash128::of("rg.doc.v1", text.as_bytes()))
}

fn visibility_of(name: &str) -> Visibility {
    if name.starts_with("__") && !name.ends_with("__") {
        Visibility::Private
    } else if name.starts_with('_') && !name.starts_with("__") {
        Visibility::Protected
    } else {
        Visibility::Public
    }
}

fn is_constant_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

impl<'a, 't> Collector<'a, 't> {
    fn visit_scope(&mut self, node: Node<'t>, parent: LocalId, scope: Scope, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            let range = self.src.range(node);
            self.sink.note(
                DiagSeverity::Warning,
                DiagCode::DepthLimit,
                "subtree skipped: nesting is too deep",
                Some(range),
            );
            return;
        }
        for stmt in children_of(node) {
            match stmt.kind() {
                "function_definition" => self.function(stmt, stmt, &[], parent, scope),
                "class_definition" => self.class(stmt, stmt, &[], parent, depth),
                "decorated_definition" => {
                    let mut decorators = Vec::new();
                    let mut cursor = stmt.walk();
                    for child in stmt.children(&mut cursor) {
                        if child.kind() == "decorator" {
                            decorators.push(child);
                        }
                    }
                    match stmt.child_by_field_name("definition") {
                        Some(def) if def.kind() == "function_definition" => {
                            self.function(def, stmt, &decorators, parent, scope);
                        }
                        Some(def) if def.kind() == "class_definition" => {
                            self.class(def, stmt, &decorators, parent, depth);
                        }
                        _ => {}
                    }
                }
                "expression_statement" => {
                    for child in children_of(stmt) {
                        if child.kind() == "assignment" {
                            self.assignment(child, stmt, parent, scope);
                        }
                    }
                }
                "if_statement" | "try_statement" | "with_statement" => {
                    self.descend(stmt, parent, scope, depth + 1);
                }
                _ => {}
            }
        }
    }

    /// Conditional definitions (`if TYPE_CHECKING:`, `try: ... except ImportError:`) belong to the
    /// enclosing scope.
    fn descend(&mut self, node: Node<'t>, parent: LocalId, scope: Scope, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            return;
        }
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.children(&mut cursor).collect();
        for child in children {
            match child.kind() {
                "block" => self.visit_scope(child, parent, scope, depth + 1),
                "elif_clause"
                | "else_clause"
                | "except_clause"
                | "except_group_clause"
                | "finally_clause" => self.descend(child, parent, scope, depth + 1),
                _ => {}
            }
        }
    }

    fn push(&mut self, mut symbol: IrSymbol, entry: Entry<'t>) -> LocalId {
        let id = LocalId(u32::try_from(self.table.symbols.len()).unwrap_or(u32::MAX));
        symbol.local_id = id;
        self.table.symbols.push(symbol);
        self.table.entries.push(entry);
        id
    }

    fn qualified(&self, parent: LocalId, name: &str) -> Vec<String> {
        if parent.0 == 0 {
            return vec![name.to_owned()];
        }
        let mut qualified = self.table.symbols[parent.0 as usize].qualified_name.clone();
        qualified.push(name.to_owned());
        qualified
    }

    fn is_exported(&self, name: &str) -> bool {
        match &self.exported {
            Some(set) => set.contains(name),
            None => !name.starts_with('_'),
        }
    }

    fn decorator(&self, node: Node<'_>) -> IrDecorator {
        let inner = children_of(node).into_iter().next();
        let (name, args) = match inner {
            Some(n) if n.kind() == "call" => (
                n.child_by_field_name("function")
                    .and_then(|f| dotted(f, &self.src))
                    .map(|p| p.join("."))
                    .unwrap_or_else(|| "?".to_owned()),
                call_args(n, &self.src, 1),
            ),
            Some(n) => (
                dotted(n, &self.src)
                    .map(|p| p.join("."))
                    .unwrap_or_else(|| collapse_ws(&self.src.text(n))),
                Vec::new(),
            ),
            None => ("?".to_owned(), Vec::new()),
        };
        IrDecorator {
            name,
            args,
            range: self.src.range(node),
        }
    }

    /// Parameters of a function, without a leading `self`/`cls` of a method, plus the default
    /// values that are calls (`Depends(...)`, `Query(...)`), which framework adapters read.
    fn params(
        &self,
        node: Option<Node<'_>>,
        scope: Scope,
    ) -> (Vec<IrParam>, BTreeMap<String, AttrValue>) {
        let mut params = Vec::new();
        let mut defaults: BTreeMap<String, AttrValue> = BTreeMap::new();
        let Some(node) = node else {
            return (params, defaults);
        };
        for (index, param) in children_of(node).into_iter().enumerate() {
            let mut name = String::new();
            let mut type_text = None;
            let mut optional = false;
            let mut rest = false;
            let mut default_value = None;
            match param.kind() {
                "identifier" => name = self.src.text(param).into_owned(),
                "typed_parameter" => {
                    for child in children_of(param) {
                        match child.kind() {
                            "identifier" => name = self.src.text(child).into_owned(),
                            "list_splat_pattern" | "dictionary_splat_pattern" => {
                                rest = true;
                                name = self.splat_name(child);
                            }
                            _ => {}
                        }
                    }
                    type_text = param
                        .child_by_field_name("type")
                        .map(|t| collapse_ws(&self.src.text(t)));
                }
                "default_parameter" | "typed_default_parameter" => {
                    optional = true;
                    if let Some(n) = param.child_by_field_name("name") {
                        name = self.src.text(n).into_owned();
                    }
                    type_text = param
                        .child_by_field_name("type")
                        .map(|t| collapse_ws(&self.src.text(t)));
                    default_value = param.child_by_field_name("value");
                }
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    rest = true;
                    name = self.splat_name(param);
                }
                _ => continue,
            }
            if name.is_empty() {
                continue;
            }
            if index == 0 && scope == Scope::Class && (name == "self" || name == "cls") {
                continue;
            }
            if let Some(value) = default_value {
                if value.kind() == "call" {
                    defaults.insert(name.clone(), AttrValue::Expr(expr(value, &self.src, 1)));
                }
            }
            params.push(IrParam {
                name,
                type_text,
                optional,
                rest,
                decorators: Vec::new(),
                property: None,
            });
        }
        (params, defaults)
    }

    fn splat_name(&self, node: Node<'_>) -> String {
        children_of(node)
            .into_iter()
            .next()
            .map(|n| self.src.text(n).into_owned())
            .unwrap_or_default()
    }

    fn function(
        &mut self,
        def: Node<'t>,
        outer: Node<'t>,
        decorators: &[Node<'t>],
        parent: LocalId,
        scope: Scope,
    ) {
        let Some(name_node) = def.child_by_field_name("name") else {
            return;
        };
        let name = self.src.text(name_node).into_owned();
        let decs: Vec<IrDecorator> = decorators.iter().map(|d| self.decorator(*d)).collect();
        let has = |wanted: &str| decs.iter().any(|d| last_segment(&d.name) == wanted);
        let is_setter = decs.iter().any(|d| d.name.ends_with(".setter"));
        let kind = match scope {
            Scope::Module => SymbolKind::Function,
            Scope::Class => {
                if name == "__init__" {
                    SymbolKind::Constructor
                } else if has("property") || has("cached_property") {
                    SymbolKind::Getter
                } else if is_setter {
                    SymbolKind::Setter
                } else {
                    SymbolKind::Method
                }
            }
        };
        let mut modifiers = Modifiers::empty();
        let mut cursor = def.walk();
        if def.children(&mut cursor).any(|c| c.kind() == "async") {
            modifiers.insert(Modifiers::ASYNC);
        }
        if has("staticmethod") || has("classmethod") {
            modifiers.insert(Modifiers::STATIC);
        }
        if has("abstractmethod") {
            modifiers.insert(Modifiers::ABSTRACT);
        }
        if scope == Scope::Module && parent.0 == 0 && self.is_exported(&name) {
            modifiers.insert(Modifiers::EXPORTED);
        }
        let body = def.child_by_field_name("body");
        let sig_end = body.map_or(def.end_byte(), |b| b.start_byte());
        let signature = collapse_ws(&self.src.slice(def.start_byte(), sig_end))
            .trim_end()
            .trim_end_matches(':')
            .trim_end()
            .chars()
            .take(MAX_SIGNATURE_CHARS)
            .collect::<String>();
        let (params, defaults) = self.params(def.child_by_field_name("parameters"), scope);
        let qualified = self.qualified(parent, &name);
        let mut symbol = IrSymbol::new(
            LocalId(0),
            kind,
            name.clone(),
            qualified,
            self.src.range(outer),
        );
        symbol.parent = Some(parent);
        symbol.name_range = Some(self.src.range(name_node));
        symbol.body_range = body.map(|b| self.src.range(b));
        symbol.signature = Some(signature);
        symbol.modifiers = modifiers;
        symbol.visibility = visibility_of(&name);
        symbol.params = params;
        symbol.return_type = def
            .child_by_field_name("return_type")
            .map(|t| collapse_ws(&self.src.text(t)));
        symbol.doc_hash = body.and_then(|b| docstring_hash(b, &self.src));
        if has("overload") {
            symbol
                .attrs
                .insert("overload".to_owned(), AttrValue::Bool(true));
        }
        if has("classmethod") {
            symbol
                .attrs
                .insert("classmethod".to_owned(), AttrValue::Bool(true));
        }
        if matches!(kind, SymbolKind::Getter | SymbolKind::Setter) {
            symbol
                .attrs
                .insert("property".to_owned(), AttrValue::Bool(true));
        }
        if !defaults.is_empty() {
            symbol
                .attrs
                .insert("param_defaults".to_owned(), AttrValue::Map(defaults));
        }
        symbol.decorators = decs;
        self.push(symbol, Entry { outer, body });
    }

    fn class(
        &mut self,
        def: Node<'t>,
        outer: Node<'t>,
        decorators: &[Node<'t>],
        parent: LocalId,
        depth: usize,
    ) {
        let Some(name_node) = def.child_by_field_name("name") else {
            return;
        };
        let name = self.src.text(name_node).into_owned();
        let decs: Vec<IrDecorator> = decorators.iter().map(|d| self.decorator(*d)).collect();
        let body = def.child_by_field_name("body");
        let mut modifiers = Modifiers::empty();
        if parent.0 == 0 && self.is_exported(&name) {
            modifiers.insert(Modifiers::EXPORTED);
        }
        let mut extends = Vec::new();
        let mut metaclass = None;
        if let Some(bases) = def.child_by_field_name("superclasses") {
            for base in children_of(bases) {
                if base.kind() == "keyword_argument" {
                    let is_meta = base
                        .child_by_field_name("name")
                        .is_some_and(|n| self.src.text(n) == "metaclass");
                    if is_meta {
                        metaclass = base
                            .child_by_field_name("value")
                            .map(|v| collapse_ws(&self.src.text(v)));
                    }
                } else {
                    extends.push(collapse_ws(&self.src.text(base)));
                }
            }
        }
        if extends.iter().any(|b| last_segment(b) == "ABC")
            || metaclass
                .as_deref()
                .is_some_and(|m| last_segment(m) == "ABCMeta")
        {
            modifiers.insert(Modifiers::ABSTRACT);
        }
        let sig_end = body.map_or(def.end_byte(), |b| b.start_byte());
        let signature = collapse_ws(&self.src.slice(def.start_byte(), sig_end))
            .trim_end()
            .trim_end_matches(':')
            .trim_end()
            .chars()
            .take(MAX_SIGNATURE_CHARS)
            .collect::<String>();
        let qualified = self.qualified(parent, &name);
        let mut symbol = IrSymbol::new(
            LocalId(0),
            SymbolKind::Class,
            name.clone(),
            qualified,
            self.src.range(outer),
        );
        symbol.parent = Some(parent);
        symbol.name_range = Some(self.src.range(name_node));
        symbol.body_range = body.map(|b| self.src.range(b));
        symbol.signature = Some(signature);
        symbol.modifiers = modifiers;
        symbol.visibility = visibility_of(&name);
        symbol.heritage.extends = extends;
        symbol.doc_hash = body.and_then(|b| docstring_hash(b, &self.src));
        if let Some(metaclass) = metaclass {
            symbol
                .attrs
                .insert("metaclass".to_owned(), AttrValue::Str(metaclass));
        }
        symbol.decorators = decs;
        let id = self.push(symbol, Entry { outer, body });
        if let Some(body) = body {
            self.visit_scope(body, id, Scope::Class, depth + 1);
        }
    }

    fn assignment(&mut self, assign: Node<'t>, stmt: Node<'t>, parent: LocalId, scope: Scope) {
        let Some(left) = assign.child_by_field_name("left") else {
            return;
        };
        let mut names: Vec<(String, Node<'t>)> = Vec::new();
        match left.kind() {
            "identifier" => names.push((self.src.text(left).into_owned(), left)),
            "pattern_list" | "tuple_pattern" | "list_pattern" => {
                for child in children_of(left) {
                    if child.kind() == "identifier" {
                        names.push((self.src.text(child).into_owned(), child));
                    }
                }
            }
            _ => {}
        }
        let destructured = left.kind() != "identifier";
        let right = assign.child_by_field_name("right");
        let type_text = assign
            .child_by_field_name("type")
            .map(|t| collapse_ws(&self.src.text(t)));
        for (name, name_node) in names {
            let kind = match scope {
                Scope::Module => {
                    if is_constant_name(&name) {
                        SymbolKind::Constant
                    } else {
                        SymbolKind::Variable
                    }
                }
                Scope::Class => SymbolKind::Field,
            };
            if !self
                .seen_vars
                .insert((parent.0, name.clone(), kind.as_id_str()))
            {
                continue;
            }
            let qualified = self.qualified(parent, &name);
            let mut symbol = IrSymbol::new(
                LocalId(0),
                kind,
                name.clone(),
                qualified,
                self.src.range(stmt),
            );
            symbol.parent = Some(parent);
            symbol.name_range = Some(self.src.range(name_node));
            symbol.body_range = right.map(|r| self.src.range(r));
            symbol.visibility = visibility_of(&name);
            symbol.declared_type = type_text.clone();
            if scope == Scope::Module && parent.0 == 0 && self.is_exported(&name) {
                symbol.modifiers.insert(Modifiers::EXPORTED);
            }
            if destructured {
                symbol
                    .attrs
                    .insert("destructured".to_owned(), AttrValue::Bool(true));
            }
            if let Some(right) = right {
                self.describe_value(&mut symbol, right, &name);
            }
            self.push(
                symbol,
                Entry {
                    outer: stmt,
                    body: right,
                },
            );
        }
    }

    /// Records the literal value of a constant and the constructor call of a field.
    fn describe_value(&self, symbol: &mut IrSymbol, right: Node<'_>, name: &str) {
        let lower = name.to_ascii_lowercase();
        let secret = SECRET_FRAGMENTS.iter().any(|f| lower.contains(f));
        if symbol.kind == SymbolKind::Constant {
            if secret {
                symbol
                    .attrs
                    .insert("redacted".to_owned(), AttrValue::Bool(true));
            } else {
                symbol.const_value = match right.kind() {
                    "string" => string_value(right, &self.src).map(ConstValue::Str),
                    "integer" | "float" => Some(ConstValue::Num(self.src.text(right).into_owned())),
                    "true" => Some(ConstValue::Bool(true)),
                    "false" => Some(ConstValue::Bool(false)),
                    _ => None,
                };
            }
        }
        if right.kind() == "call" && !secret {
            if let IrExpr::Call { callee, args } = expr(right, &self.src, 1) {
                symbol
                    .attrs
                    .insert("init_call".to_owned(), AttrValue::Str(callee.join(".")));
                match args.first() {
                    Some(IrExpr::Str(value)) | Some(IrExpr::Ident(value)) => {
                        symbol
                            .attrs
                            .insert("init_arg0".to_owned(), AttrValue::Str(value.clone()));
                    }
                    Some(IrExpr::Member(parts)) => {
                        symbol
                            .attrs
                            .insert("init_arg0".to_owned(), AttrValue::Str(parts.join(".")));
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Marks symbols whose range overlaps an error range.
pub fn mark_errors(symbols: &mut [IrSymbol], error_ranges: &[review_core::location::SourceRange]) {
    if error_ranges.is_empty() {
        return;
    }
    for symbol in symbols {
        if error_ranges
            .iter()
            .any(|e| symbol.range.start <= e.end && e.start <= symbol.range.end)
        {
            symbol.has_errors = true;
        }
    }
}
