//! Reference extraction (TSA-005): calls, `new`, type references, heritage, decorators, JSX
//! elements and reads of imported bindings, each with a receiver hint.
//!
//! One bounded recursive walk over the tree. A reference belongs to the innermost symbol whose
//! range contains it, so callbacks and anonymous functions (not symbols under the default
//! policy) fold into the enclosing symbol with `attrs.in_callback`, and module-level statements
//! land on the module symbol. Output is in source order.

use std::collections::BTreeMap;

use analysis_ir::reference::BindingRef;
use analysis_ir::{
    AttrValue, DiagCode, DiagSeverity, IrImport, IrReference, IrSymbol, LocalId, ReceiverHint,
    RefKind,
};
use review_core::location::{Position, SourceRange};
use tree_sitter::Node;

use super::receiver::{unwrap_expr, Callee, Resolver};
use super::type_env::{build_class_env, declared_type_of, new_target_text, TypeEnv};
use super::{depth_limit, MAX_VISIT_DEPTH};
use crate::bindings::BindingTable;
use crate::diagnostics::DiagnosticSink;
use crate::kinds::{field, kind};
use crate::text::Source;

/// References kept per file; beyond this one info diagnostic is emitted and the rest are dropped.
pub const MAX_REFERENCES_PER_FILE: usize = 50_000;

const TEST_CALLEES: [&str; 4] = ["describe", "it", "test", "suite"];

const CLASS_STATIC_BLOCK: &str = "class_static_block";
const JSX_CLOSING_ELEMENT: &str = "jsx_closing_element";

/// Extracts the references of every symbol. `symbols` must be the unit's symbols in order.
pub fn collect(
    root: Node<'_>,
    src: &Source<'_>,
    symbols: &[IrSymbol],
    imports: &[IrImport],
    bindings: &BindingTable,
    sink: &mut DiagnosticSink,
) -> Vec<IrReference> {
    let mut walker = Walker {
        symbols,
        resolver: Resolver {
            src,
            imports,
            bindings,
        },
        env: TypeEnv::new(),
        out: Vec::new(),
        test_depth: 0,
        callback_depth: 0,
        depth_reported: false,
        capped: false,
        sink,
    };
    walker.visit(root, 0);
    let mut out = walker.out;
    // Stable: references with equal positions keep their walk order.
    out.sort_by_key(|reference| reference.range.start);
    out
}

struct Walker<'s, 'a> {
    symbols: &'s [IrSymbol],
    resolver: Resolver<'s, 'a>,
    env: TypeEnv,
    out: Vec<IrReference>,
    test_depth: u32,
    callback_depth: u32,
    depth_reported: bool,
    capped: bool,
    sink: &'s mut DiagnosticSink,
}

fn contains(range: &SourceRange, at: Position) -> bool {
    range.start <= at && at <= range.end
}

fn is_capitalized(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

fn count_arguments(arguments: Option<Node<'_>>) -> u16 {
    let Some(arguments) = arguments else { return 0 };
    if arguments.kind() != kind::ARGUMENTS {
        return 1;
    }
    let mut cursor = arguments.walk();
    let count = arguments
        .named_children(&mut cursor)
        .filter(|c| c.kind() != kind::COMMENT)
        .count();
    u16::try_from(count).unwrap_or(u16::MAX)
}

impl<'s, 'a> Walker<'s, 'a> {
    fn src(&self) -> &'s Source<'a> {
        self.resolver.src
    }

    fn text(&self, node: Node<'_>) -> String {
        self.src().text(node).into_owned()
    }

    /// The innermost symbol whose range contains `at`: the latest start, then the earliest end.
    fn owner(&self, at: Position) -> LocalId {
        let mut best: Option<&IrSymbol> = None;
        for symbol in self.symbols {
            if !contains(&symbol.range, at) {
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
        best.map(|symbol| symbol.local_id).unwrap_or(LocalId(0))
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        range: SourceRange,
        ref_kind: RefKind,
        name: String,
        receiver: ReceiverHint,
        import_binding: Option<BindingRef>,
        arg_count: u16,
        mut attrs: BTreeMap<String, AttrValue>,
    ) {
        if self.out.len() >= MAX_REFERENCES_PER_FILE {
            if !self.capped {
                self.capped = true;
                self.sink.note(
                    DiagSeverity::Info,
                    DiagCode::UnsupportedConstruct,
                    "too many references in one file; the rest were dropped",
                    None,
                );
            }
            return;
        }
        if self.callback_depth > 0 {
            attrs.insert("in_callback".to_owned(), AttrValue::Bool(true));
        }
        let from = self.owner(range.start);
        self.out.push(IrReference {
            from,
            kind: ref_kind,
            name,
            receiver,
            import_binding,
            range,
            arg_count,
            in_test_block: self.test_depth > 0,
            attrs,
        });
    }

    fn emit_callee(&mut self, ref_kind: RefKind, callee: Callee, arg_count: u16) {
        self.emit(
            callee.name_range,
            ref_kind,
            callee.name,
            callee.receiver,
            callee.import_binding,
            arg_count,
            callee.attrs,
        );
    }

    fn children(&mut self, node: Node<'_>, depth: usize) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child, depth + 1);
        }
    }

    fn visit(&mut self, node: Node<'_>, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            if !self.depth_reported {
                self.depth_reported = true;
                let range = self.src().range(node);
                depth_limit(self.sink, range);
            }
            return;
        }
        match node.kind() {
            kind::COMMENT | kind::IMPORT_STATEMENT | kind::EXPORT_CLAUSE | JSX_CLOSING_ELEMENT => {}
            kind::CLASS_DECLARATION | kind::ABSTRACT_CLASS_DECLARATION | kind::CLASS => {
                self.class(node, depth);
            }
            kind::INTERFACE_DECLARATION => self.interface(node, depth),
            kind::TYPE_ALIAS_DECLARATION => self.type_alias(node, depth),
            kind::ENUM_DECLARATION | kind::INTERNAL_MODULE | kind::MODULE => {
                if let Some(body) = node.child_by_field_name(field::BODY) {
                    self.visit(body, depth + 1);
                }
            }
            kind::FUNCTION_DECLARATION
            | kind::GENERATOR_FUNCTION_DECLARATION
            | kind::FUNCTION_SIGNATURE
            | kind::FUNCTION_EXPRESSION
            | kind::GENERATOR_FUNCTION
            | kind::ARROW_FUNCTION
            | kind::METHOD_DEFINITION
            | kind::METHOD_SIGNATURE
            | kind::ABSTRACT_METHOD_SIGNATURE => self.function(node, depth),
            kind::PUBLIC_FIELD_DEFINITION => self.field_definition(node, depth),
            kind::STATEMENT_BLOCK
            | kind::FOR_STATEMENT
            | kind::SWITCH_STATEMENT
            | CLASS_STATIC_BLOCK => {
                self.env.push_scope();
                self.children(node, depth);
                self.env.pop_scope();
            }
            kind::FOR_IN_STATEMENT => {
                self.env.push_scope();
                if let Some(left) = node.child_by_field_name(field::LEFT) {
                    self.declare_pattern(left, None, depth + 1);
                }
                self.children(node, depth);
                self.env.pop_scope();
            }
            kind::CATCH_CLAUSE => {
                self.env.push_scope();
                if let Some(parameter) = node.child_by_field_name(field::PARAMETER) {
                    self.declare_pattern(parameter, None, depth + 1);
                }
                if let Some(body) = node.child_by_field_name(field::BODY) {
                    self.visit(body, depth + 1);
                }
                self.env.pop_scope();
            }
            kind::VARIABLE_DECLARATOR => self.declarator(node, depth),
            kind::CALL_EXPRESSION => self.call(node, RefKind::Call, depth),
            kind::NEW_EXPRESSION => self.call(node, RefKind::New, depth),
            kind::DECORATOR => self.decorator(node, depth),
            kind::TYPE_IDENTIFIER
            | kind::GENERIC_TYPE
            | kind::NESTED_TYPE_IDENTIFIER
            | kind::PREDEFINED_TYPE => self.type_ref(node, RefKind::TypeRef, None, depth),
            kind::JSX_OPENING_ELEMENT | kind::JSX_SELF_CLOSING_ELEMENT => self.jsx(node, depth),
            kind::IDENTIFIER | kind::SHORTHAND_PROPERTY_IDENTIFIER => self.value_read(node),
            kind::MEMBER_EXPRESSION => {
                if let Some(object) = node.child_by_field_name(field::OBJECT) {
                    self.visit(object, depth + 1);
                }
            }
            kind::PAIR => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    let is_key = node
                        .child_by_field_name(field::KEY)
                        .is_some_and(|k| k.id() == child.id());
                    if !is_key || child.kind() == kind::COMPUTED_PROPERTY_NAME {
                        self.visit(child, depth + 1);
                    }
                }
            }
            _ => self.children(node, depth),
        }
    }

    // ----- declarations ------------------------------------------------------------------

    fn class(&mut self, node: Node<'_>, depth: usize) {
        let mark = self.env.generics_mark();
        let fields = node
            .child_by_field_name(field::BODY)
            .map(|body| build_class_env(body, self.src()))
            .unwrap_or_default();
        self.env.push_class(fields);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                kind::DECORATOR => self.decorator(child, depth + 1),
                kind::TYPE_PARAMETERS => self.type_parameters(child, depth + 1),
                kind::CLASS_HERITAGE => self.heritage(child, depth + 1),
                kind::CLASS_BODY => self.children(child, depth + 1),
                _ => {}
            }
        }
        self.env.pop_class();
        self.env.truncate_generics(mark);
    }

    fn heritage(&mut self, node: Node<'_>, depth: usize) {
        let mut cursor = node.walk();
        for clause in node.named_children(&mut cursor) {
            match clause.kind() {
                kind::EXTENDS_CLAUSE => self.extends_clause(clause, depth + 1),
                kind::IMPLEMENTS_CLAUSE => {
                    let mut inner = clause.walk();
                    for ty in clause.named_children(&mut inner) {
                        if ty.kind() != kind::COMMENT {
                            self.type_ref(ty, RefKind::Implements, None, depth + 1);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// `extends Base`, `extends ns.Base`, `extends AuthGuard('jwt')` and `extends Base<T>`.
    fn extends_clause(&mut self, clause: Node<'_>, depth: usize) {
        let mut head: Option<String> = None;
        let mut cursor = clause.walk();
        for child in clause.named_children(&mut cursor) {
            match child.kind() {
                kind::COMMENT => {}
                kind::TYPE_ARGUMENTS => {
                    self.type_arguments(child, head.as_deref(), depth + 1);
                }
                _ => {
                    let value = unwrap_expr(child);
                    if value.kind() == kind::CALL_EXPRESSION {
                        let function = value.child_by_field_name(field::FUNCTION);
                        let arguments = value.child_by_field_name(field::ARGUMENTS);
                        if let Some(mut callee) =
                            function.and_then(|f| self.resolver.classify(&self.env, f, false))
                        {
                            head = Some(callee.name.clone());
                            callee
                                .attrs
                                .insert("extends_call".to_owned(), AttrValue::Bool(true));
                            let count = count_arguments(arguments);
                            self.emit_callee(RefKind::Extends, callee, count);
                        }
                        if let Some(arguments) = arguments {
                            self.visit(arguments, depth + 1);
                        }
                    } else if let Some(callee) = self.resolver.classify(&self.env, value, false) {
                        head = Some(callee.name.clone());
                        self.emit_callee(RefKind::Extends, callee, 0);
                    }
                }
            }
        }
    }

    fn interface(&mut self, node: Node<'_>, depth: usize) {
        let mark = self.env.generics_mark();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                kind::TYPE_PARAMETERS => self.type_parameters(child, depth + 1),
                kind::EXTENDS_TYPE_CLAUSE => {
                    let mut inner = child.walk();
                    for ty in child.named_children(&mut inner) {
                        if ty.kind() != kind::COMMENT {
                            self.type_ref(ty, RefKind::Extends, None, depth + 1);
                        }
                    }
                }
                kind::INTERFACE_BODY => self.children(child, depth + 1),
                _ => {}
            }
        }
        self.env.truncate_generics(mark);
    }

    fn type_alias(&mut self, node: Node<'_>, depth: usize) {
        let mark = self.env.generics_mark();
        if let Some(params) = node.child_by_field_name(field::TYPE_PARAMETERS) {
            self.type_parameters(params, depth + 1);
        }
        if let Some(value) = node.child_by_field_name(field::VALUE) {
            self.visit(value, depth + 1);
        }
        self.env.truncate_generics(mark);
    }

    /// Declares the type parameter names (they are not references) and visits constraints and
    /// defaults.
    fn type_parameters(&mut self, node: Node<'_>, depth: usize) {
        let mut cursor = node.walk();
        let params: Vec<Node<'_>> = node
            .named_children(&mut cursor)
            .filter(|p| p.kind() == kind::TYPE_PARAMETER)
            .collect();
        for param in &params {
            if let Some(name) = param.child_by_field_name(field::NAME) {
                let name = self.text(name);
                self.env.add_generic(name);
            }
        }
        for param in params {
            let name_id = param.child_by_field_name(field::NAME).map(|n| n.id());
            let mut inner = param.walk();
            for child in param.children(&mut inner) {
                if Some(child.id()) != name_id {
                    self.visit(child, depth + 1);
                }
            }
        }
    }

    fn function(&mut self, node: Node<'_>, depth: usize) {
        let mark = self.env.generics_mark();
        let parent_kind = node.parent().map(|p| p.kind());
        let anonymous = matches!(
            node.kind(),
            kind::ARROW_FUNCTION | kind::FUNCTION_EXPRESSION | kind::GENERATOR_FUNCTION
        ) && !matches!(
            parent_kind,
            Some(
                kind::VARIABLE_DECLARATOR
                    | kind::PUBLIC_FIELD_DEFINITION
                    | kind::PAIR
                    | kind::EXPORT_STATEMENT
            )
        );
        if anonymous {
            self.callback_depth += 1;
        }
        self.env.push_scope();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == kind::DECORATOR {
                self.decorator(child, depth + 1);
            }
        }
        if let Some(type_params) = node.child_by_field_name(field::TYPE_PARAMETERS) {
            self.type_parameters(type_params, depth + 1);
        }
        if let Some(params) = node.child_by_field_name(field::PARAMETERS) {
            let mut inner = params.walk();
            for param in params.named_children(&mut inner) {
                self.parameter(param, depth + 1);
            }
        } else if let Some(param) = node.child_by_field_name(field::PARAMETER) {
            // `x => ...`
            self.declare_pattern(param, None, depth + 1);
        }
        if let Some(return_type) = node.child_by_field_name(field::RETURN_TYPE) {
            self.visit(return_type, depth + 1);
        }
        if let Some(body) = node.child_by_field_name(field::BODY) {
            self.visit(body, depth + 1);
        }
        self.env.pop_scope();
        self.env.truncate_generics(mark);
        if anonymous {
            self.callback_depth -= 1;
        }
    }

    fn parameter(&mut self, param: Node<'_>, depth: usize) {
        match param.kind() {
            kind::COMMENT => {}
            kind::REQUIRED_PARAMETER | kind::OPTIONAL_PARAMETER => {
                let pattern = param.child_by_field_name(field::PATTERN);
                let pattern_id = pattern.map(|p| p.id());
                let mut cursor = param.walk();
                for child in param.children(&mut cursor) {
                    if Some(child.id()) != pattern_id {
                        self.visit(child, depth + 1);
                    }
                }
                if let Some(pattern) = pattern {
                    let declared = declared_type_of(param, self.src());
                    let declared = if pattern.kind() == kind::IDENTIFIER {
                        declared
                    } else {
                        None
                    };
                    self.declare_pattern(pattern, declared, depth + 1);
                }
            }
            _ => self.declare_pattern(param, None, depth + 1),
        }
    }

    fn field_definition(&mut self, node: Node<'_>, depth: usize) {
        let name_id = node.child_by_field_name(field::NAME).map(|n| n.id());
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Some(child.id()) != name_id {
                self.visit(child, depth + 1);
            }
        }
    }

    fn declarator(&mut self, node: Node<'_>, depth: usize) {
        let name = node.child_by_field_name(field::NAME);
        let value = node.child_by_field_name(field::VALUE);
        if let Some(annotation) = node.child_by_field_name(field::TYPE) {
            self.visit(annotation, depth + 1);
        }
        if let Some(value) = value {
            self.visit(value, depth + 1);
        }
        let Some(name) = name else { return };
        let declared = if name.kind() == kind::IDENTIFIER {
            declared_type_of(node, self.src())
                .or_else(|| value.and_then(|v| new_target_text(v, self.src())))
        } else {
            None
        };
        self.declare_pattern(name, declared, depth + 1);
    }

    /// Declares every identifier a binding pattern introduces. Default values inside the pattern
    /// are visited as expressions.
    fn declare_pattern(&mut self, node: Node<'_>, declared: Option<String>, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            return;
        }
        match node.kind() {
            kind::IDENTIFIER | kind::SHORTHAND_PROPERTY_IDENTIFIER_PATTERN => {
                let name = self.text(node);
                self.env.declare(&name, declared);
            }
            kind::ASSIGNMENT_PATTERN | kind::OBJECT_ASSIGNMENT_PATTERN => {
                let left = node
                    .child_by_field_name(field::LEFT)
                    .or_else(|| node.named_child(0));
                let right = node.child_by_field_name(field::RIGHT);
                if let Some(right) = right {
                    self.visit(right, depth + 1);
                }
                if let Some(left) = left {
                    self.declare_pattern(left, declared, depth + 1);
                }
            }
            kind::PAIR_PATTERN => {
                if let Some(value) = node.child_by_field_name(field::VALUE) {
                    self.declare_pattern(value, None, depth + 1);
                }
            }
            kind::OBJECT_PATTERN | kind::ARRAY_PATTERN | kind::REST_PATTERN => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.declare_pattern(child, None, depth + 1);
                }
            }
            _ => {}
        }
    }

    // ----- expressions -------------------------------------------------------------------

    fn call(&mut self, node: Node<'_>, ref_kind: RefKind, depth: usize) {
        let callee_field = if ref_kind == RefKind::New {
            field::CONSTRUCTOR
        } else {
            field::FUNCTION
        };
        let callee_node = node.child_by_field_name(callee_field);
        let arguments = node.child_by_field_name(field::ARGUMENTS);
        let count = count_arguments(arguments);
        let optional_call = super::receiver::has_optional_chain(node);
        let mut is_test_call = false;
        if let Some(callee_node) = callee_node {
            if let Some(callee) = self
                .resolver
                .classify(&self.env, callee_node, optional_call)
            {
                if ref_kind == RefKind::Call {
                    is_test_call = is_test_callee(&callee);
                }
                self.emit_callee(ref_kind, callee, count);
            }
            self.callee_parts(callee_node, depth + 1);
        }
        if let Some(type_arguments) = node.child_by_field_name(field::TYPE_ARGUMENTS) {
            self.type_arguments(type_arguments, None, depth + 1);
        }
        if let Some(arguments) = arguments {
            if is_test_call {
                self.test_depth += 1;
            }
            self.visit(arguments, depth + 1);
            if is_test_call {
                self.test_depth -= 1;
            }
        }
    }

    /// Visits what a callee contains besides the terminal name: nested calls in a builder chain,
    /// computed keys, an immediately invoked function. The root identifier of a receiver chain is
    /// covered by the receiver hint, so it is not read again.
    fn callee_parts(&mut self, node: Node<'_>, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            return;
        }
        let node = unwrap_expr(node);
        match node.kind() {
            kind::IDENTIFIER | kind::THIS | kind::SUPER | kind::IMPORT => {}
            kind::MEMBER_EXPRESSION => {
                if let Some(object) = node.child_by_field_name(field::OBJECT) {
                    self.receiver_parts(object, depth + 1);
                }
            }
            _ => self.visit(node, depth + 1),
        }
    }

    fn receiver_parts(&mut self, node: Node<'_>, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            return;
        }
        let node = unwrap_expr(node);
        match node.kind() {
            kind::IDENTIFIER | kind::THIS | kind::SUPER => {}
            kind::MEMBER_EXPRESSION => {
                if let Some(object) = node.child_by_field_name(field::OBJECT) {
                    self.receiver_parts(object, depth + 1);
                }
            }
            _ => self.visit(node, depth + 1),
        }
    }

    fn value_read(&mut self, node: Node<'_>) {
        let name = self.text(node);
        let Some(binding) = self.resolver.binding_for(&self.env, &name) else {
            return;
        };
        let range = self.src().range(node);
        self.emit(
            range,
            RefKind::ValueRead,
            name,
            ReceiverHint::None,
            Some(binding),
            0,
            BTreeMap::new(),
        );
    }

    fn decorator(&mut self, node: Node<'_>, depth: usize) {
        let Some(expr) = node.named_child(0) else {
            return;
        };
        let expr = unwrap_expr(expr);
        if expr.kind() == kind::CALL_EXPRESSION {
            let arguments = expr.child_by_field_name(field::ARGUMENTS);
            let count = count_arguments(arguments);
            if let Some(function) = expr.child_by_field_name(field::FUNCTION) {
                if let Some(callee) = self.resolver.classify(&self.env, function, false) {
                    self.emit_callee(RefKind::Decorator, callee, count);
                }
                self.callee_parts(function, depth + 1);
            }
            if let Some(arguments) = arguments {
                self.visit(arguments, depth + 1);
            }
        } else if let Some(callee) = self.resolver.classify(&self.env, expr, false) {
            self.emit_callee(RefKind::Decorator, callee, 0);
        }
    }

    // ----- types and JSX -----------------------------------------------------------------

    fn type_arguments(&mut self, node: Node<'_>, arg_of: Option<&str>, depth: usize) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != kind::COMMENT {
                self.type_ref(child, RefKind::TypeRef, arg_of, depth + 1);
            }
        }
    }

    /// Emits the reference a type node stands for, descending into generics. Other type
    /// constructs (unions, arrays, function types, ...) are walked for the types inside them.
    fn type_ref(&mut self, node: Node<'_>, ref_kind: RefKind, arg_of: Option<&str>, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            return;
        }
        let mut attrs: BTreeMap<String, AttrValue> = BTreeMap::new();
        if let Some(parent) = arg_of {
            attrs.insert("type_arg_of".to_owned(), AttrValue::Str(parent.to_owned()));
        }
        match node.kind() {
            kind::TYPE_IDENTIFIER => {
                let name = self.text(node);
                if self.env.is_generic(&name) {
                    return;
                }
                let binding = self.resolver.binding_for(&self.env, &name);
                let range = self.src().range(node);
                self.emit(range, ref_kind, name, ReceiverHint::None, binding, 0, attrs);
            }
            kind::PREDEFINED_TYPE => {
                let name = self.text(node);
                let range = self.src().range(node);
                self.emit(range, ref_kind, name, ReceiverHint::None, None, 0, attrs);
            }
            kind::NESTED_TYPE_IDENTIFIER => {
                let (Some(module), Some(name)) = (
                    node.child_by_field_name("module"),
                    node.child_by_field_name(field::NAME),
                ) else {
                    return;
                };
                let Some(parts) = self.resolver.parts_of(module) else {
                    return;
                };
                let hint = self.resolver.path_hint(&self.env, &parts);
                let text = self.text(name);
                let range = self.src().range(name);
                self.emit(range, ref_kind, text, hint.receiver, hint.binding, 0, attrs);
            }
            kind::GENERIC_TYPE => {
                let head = node.child_by_field_name(field::NAME);
                let head_name = head.map(|h| terminal_name(&self.text(h)));
                if let Some(head) = head {
                    self.type_ref(head, ref_kind, arg_of, depth + 1);
                }
                if let Some(arguments) = node.child_by_field_name(field::TYPE_ARGUMENTS) {
                    self.type_arguments(arguments, head_name.as_deref(), depth + 1);
                }
            }
            _ => self.children(node, depth),
        }
    }

    fn jsx(&mut self, node: Node<'_>, depth: usize) {
        let name_node = node.child_by_field_name(field::NAME);
        if let Some(name_node) = name_node {
            if let Some(parts) = jsx_parts(&self.resolver, name_node) {
                if parts.first().is_some_and(|p| is_capitalized(p)) {
                    let last = parts.last().cloned().unwrap_or_default();
                    let root = &parts[..parts.len().saturating_sub(1)];
                    let hint = if root.is_empty() {
                        self.resolver.root_hint(&self.env, &last)
                    } else {
                        self.resolver.path_hint(&self.env, root)
                    };
                    let mut attrs = BTreeMap::new();
                    if root.is_empty() && hint.static_candidate {
                        attrs.insert("static_candidate".to_owned(), AttrValue::Bool(true));
                    }
                    let (receiver, binding) = if root.is_empty() {
                        (ReceiverHint::None, hint.binding)
                    } else {
                        (hint.receiver, hint.binding)
                    };
                    let range = self.src().range(name_node);
                    self.emit(
                        range,
                        RefKind::JsxElement,
                        last,
                        receiver,
                        binding,
                        0,
                        attrs,
                    );
                }
            }
        }
        let name_id = name_node.map(|n| n.id());
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Some(child.id()) != name_id {
                self.visit(child, depth + 1);
            }
        }
    }
}

/// The dotted parts of a JSX element name; namespaced names (`a:b`) are not references.
fn jsx_parts(resolver: &Resolver<'_, '_>, node: Node<'_>) -> Option<Vec<String>> {
    match node.kind() {
        kind::IDENTIFIER | kind::NESTED_IDENTIFIER | kind::MEMBER_EXPRESSION => {
            resolver.parts_of(node)
        }
        _ => None,
    }
}

fn terminal_name(text: &str) -> String {
    text.rsplit('.').next().unwrap_or(text).trim().to_owned()
}

fn is_test_callee(callee: &Callee) -> bool {
    match &callee.receiver {
        ReceiverHint::None => TEST_CALLEES.contains(&callee.name.as_str()),
        ReceiverHint::Identifier { name, .. } => TEST_CALLEES.contains(&name.as_str()),
        _ => false,
    }
}
