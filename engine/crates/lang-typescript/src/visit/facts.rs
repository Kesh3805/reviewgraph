//! Per-symbol syntax facts (TSA-006): a position-free summary of what each symbol does, which the
//! change classifier compares between base and head.
//!
//! One walk over the tree routes every interesting node to the innermost enclosing symbol (by
//! range). Callbacks and anonymous functions are not symbols under the default
//! `AnonymousFnPolicy::Attribute`, so their facts fold into the enclosing symbol; module-level
//! statements land on the module symbol. Decorator facts come from the symbols' own decorators.

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::symbol::FloatBits;
use analysis_ir::{
    AnalyzerConfig, AttrValue, DiagCode, DiagSeverity, FactKind, IrExpr, IrSymbol, LocalId,
    SymbolFacts, SyntaxFact,
};
use review_core::location::{Position, SourceRange};
use review_core::symbol::SymbolKind;
use tree_sitter::Node;

use crate::diagnostics::DiagnosticSink;
use crate::kinds::{field, kind};
use crate::text::Source;
use crate::visit::db_heuristics::{self, DbAccess};
use crate::visit::fact_keys::{
    argc, call_key, callee_of, h8, h8_text, is_guard_name, new_key, Callee,
};
use crate::visit::MAX_VISIT_DEPTH;

/// Facts kept per symbol; beyond this one info diagnostic is emitted and the rest are dropped.
pub const MAX_FACTS_PER_SYMBOL: usize = 2_000;

/// Identifiers kept in a condition's `idents` detail.
const MAX_CONDITION_IDENTS: usize = 8;

/// Bound for the small sub-walks (await inside a loop, throw inside a catch, identifiers).
const MAX_SCAN_NODES: usize = 4_096;

/// Extracts the facts of every symbol. Returns groups sorted by `LocalId`, facts in source order;
/// symbols without facts are omitted.
pub fn collect(
    root: Node<'_>,
    src: &Source<'_>,
    symbols: &[IrSymbol],
    cfg: &AnalyzerConfig,
    sink: &mut DiagnosticSink,
) -> Vec<SymbolFacts> {
    let mut walker = Walker {
        src,
        symbols,
        types: receiver_types(symbols),
        out: BTreeMap::new(),
        seq: 0,
    };
    walker.decorators(cfg);
    walker.walk(root, 0, false);
    walker.finish(sink)
}

/// Declared types of fields, properties and parameters by name, for the receiver test.
fn receiver_types(symbols: &[IrSymbol]) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for symbol in symbols {
        if matches!(
            symbol.kind,
            SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable | SymbolKind::Constant
        ) {
            if let Some(declared) = &symbol.declared_type {
                out.entry(symbol.name.clone())
                    .or_insert_with(|| declared.clone());
            }
        }
        for param in &symbol.params {
            if let Some(declared) = &param.type_text {
                out.entry(param.name.clone())
                    .or_insert_with(|| declared.clone());
            }
        }
    }
    out
}

struct Pending {
    start: Position,
    seq: u64,
    fact: SyntaxFact,
}

struct Walker<'s, 'a> {
    src: &'s Source<'a>,
    symbols: &'s [IrSymbol],
    types: BTreeMap<String, String>,
    out: BTreeMap<u32, Vec<Pending>>,
    seq: u64,
}

fn contains(range: &SourceRange, at: Position) -> bool {
    range.start <= at && at <= range.end
}

impl Walker<'_, '_> {
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

    fn push_to(&mut self, owner: LocalId, fact: SyntaxFact) {
        self.seq += 1;
        self.out.entry(owner.0).or_default().push(Pending {
            start: fact.range.start,
            seq: self.seq,
            fact,
        });
    }

    fn push(
        &mut self,
        node: Node<'_>,
        kind: FactKind,
        key: String,
        mut detail: BTreeMap<String, AttrValue>,
        in_tx: bool,
    ) {
        if in_tx {
            detail.insert("in_transaction".to_owned(), AttrValue::Bool(true));
        }
        let range = self.src.range(node);
        let owner = self.owner(range.start);
        self.push_to(
            owner,
            SyntaxFact {
                kind,
                key,
                range,
                detail,
            },
        );
    }

    fn finish(self, sink: &mut DiagnosticSink) -> Vec<SymbolFacts> {
        let mut groups: Vec<SymbolFacts> = Vec::with_capacity(self.out.len());
        for (symbol, mut pending) in self.out {
            pending.sort_by(|a, b| a.start.cmp(&b.start).then(a.seq.cmp(&b.seq)));
            if pending.len() > MAX_FACTS_PER_SYMBOL {
                pending.truncate(MAX_FACTS_PER_SYMBOL);
                sink.note(
                    DiagSeverity::Info,
                    DiagCode::UnsupportedConstruct,
                    "too many syntax facts in one symbol; the rest were dropped",
                    None,
                );
            }
            groups.push(SymbolFacts {
                symbol: LocalId(symbol),
                facts: pending.into_iter().map(|p| p.fact).collect(),
            });
        }
        groups
    }

    /// `GuardDecorator` and `@Transactional` facts from the symbols' decorators.
    fn decorators(&mut self, cfg: &AnalyzerConfig) {
        let configured = cfg.guard_decorator_names.as_deref();
        let mut found: Vec<(LocalId, SyntaxFact)> = Vec::new();
        for symbol in self.symbols {
            for decorator in &symbol.decorators {
                let name = decorator
                    .name
                    .rsplit('.')
                    .next()
                    .unwrap_or(decorator.name.as_str());
                if name == "Transactional" {
                    found.push((
                        symbol.local_id,
                        SyntaxFact {
                            kind: FactKind::TransactionWrapper,
                            key: "tx:@Transactional".to_owned(),
                            range: decorator.range,
                            detail: BTreeMap::new(),
                        },
                    ));
                }
                if is_guard_name(name, configured) {
                    let mut detail = BTreeMap::new();
                    detail.insert("name".to_owned(), AttrValue::Str(name.to_owned()));
                    found.push((
                        symbol.local_id,
                        SyntaxFact {
                            kind: FactKind::GuardDecorator,
                            key: format!("guard:{name}:{}", h8_text(&args_text(&decorator.args))),
                            range: decorator.range,
                            detail,
                        },
                    ));
                }
            }
        }
        for (owner, fact) in found {
            self.push_to(owner, fact);
        }
    }

    fn walk(&mut self, node: Node<'_>, depth: usize, in_tx: bool) {
        if depth > MAX_VISIT_DEPTH || node.is_missing() {
            return;
        }
        let tx_call = self.visit(node, in_tx);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let child_tx = in_tx || (tx_call && child.kind() == kind::ARGUMENTS);
            self.walk(child, depth + 1, child_tx);
        }
    }

    /// Emits the facts of one node. Returns true for a transaction-wrapper call, whose arguments
    /// are walked with `in_transaction` set.
    fn visit(&mut self, node: Node<'_>, in_tx: bool) -> bool {
        match node.kind() {
            kind::CALL_EXPRESSION => return self.call(node, in_tx),
            kind::NEW_EXPRESSION => self.new_expr(node, in_tx),
            kind::IF_STATEMENT => self.condition(node, "if", in_tx),
            kind::TERNARY_EXPRESSION => self.condition(node, "ternary", in_tx),
            kind::SWITCH_STATEMENT => self.switch(node, in_tx),
            kind::FOR_STATEMENT | kind::WHILE_STATEMENT | kind::DO_STATEMENT => {
                self.loop_stmt(node, in_tx)
            }
            kind::FOR_IN_STATEMENT => self.for_in(node, in_tx),
            kind::THROW_STATEMENT => self.throw(node, in_tx),
            kind::TRY_STATEMENT => self.try_catch(node, in_tx),
            kind::AWAIT_EXPRESSION => self.await_expr(node, in_tx),
            kind::RETURN_STATEMENT => self.return_stmt(node, in_tx),
            kind::ASSIGNMENT_EXPRESSION | kind::AUGMENTED_ASSIGNMENT_EXPRESSION => {
                self.assignment(node, in_tx)
            }
            kind::MEMBER_EXPRESSION | kind::SUBSCRIPT_EXPRESSION => self.env_read(node, in_tx),
            kind::VARIABLE_DECLARATOR => self.env_destructure(node, in_tx),
            _ => {}
        }
        false
    }

    fn call(&mut self, node: Node<'_>, in_tx: bool) -> bool {
        let Some(function) = node.child_by_field_name(field::FUNCTION) else {
            return false;
        };
        let Some(callee) = callee_of(function, self.src) else {
            return false;
        };
        let arguments = node.child_by_field_name(field::ARGUMENTS);
        let count = argc(arguments);
        let mut detail = BTreeMap::new();
        if is_validation(&callee) {
            detail.insert("validation".to_owned(), AttrValue::Bool(true));
        }
        self.push(
            node,
            FactKind::Call,
            call_key(&callee, count),
            detail,
            in_tx,
        );

        if callee.name == "forEach" {
            let header = h8_text(&callee.receiver.join("."));
            let mut detail = BTreeMap::new();
            detail.insert(
                "awaits_inside".to_owned(),
                AttrValue::Bool(
                    arguments.is_some_and(|a| contains_kind(a, kind::AWAIT_EXPRESSION)),
                ),
            );
            self.push(
                node,
                FactKind::Loop,
                format!("loop:foreach:{header}"),
                detail,
                in_tx,
            );
        }

        let first = arguments.and_then(first_argument);
        let first_string = first
            .filter(|arg| arg.kind() == kind::STRING || arg.kind() == kind::TEMPLATE_STRING)
            .map(|arg| {
                self.src
                    .text(arg)
                    .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                    .to_owned()
            });
        let first_ident = first
            .filter(|arg| arg.kind() == kind::IDENTIFIER)
            .map(|arg| self.src.text(arg).into_owned());
        let receiver_type = callee
            .receiver_name()
            .and_then(|name| self.types.get(name))
            .cloned();
        if let Some(db) = db_heuristics::classify(
            &callee,
            receiver_type.as_deref(),
            first_string.as_deref(),
            first_ident.as_deref(),
        ) {
            let entity = db.entity.clone().unwrap_or_else(|| "?".to_owned());
            let (fact_kind, prefix) = match db.access {
                DbAccess::Write => (FactKind::DbWriteLike, "dbw"),
                DbAccess::Read => (FactKind::DbReadLike, "dbr"),
            };
            let mut detail = BTreeMap::new();
            detail.insert("method".to_owned(), AttrValue::Str(db.method.clone()));
            if let Some(entity) = &db.entity {
                detail.insert("entity".to_owned(), AttrValue::Str(entity.clone()));
            }
            if let Some(declared) = &receiver_type {
                detail.insert(
                    "receiver_declared_type".to_owned(),
                    AttrValue::Str(declared.clone()),
                );
            }
            detail.insert(
                "confidence".to_owned(),
                AttrValue::Float(FloatBits::new(db.evidence.confidence())),
            );
            self.push(
                node,
                fact_kind,
                format!("{prefix}:{}:{entity}", db.method),
                detail,
                in_tx,
            );
        }

        if is_transaction(&callee) {
            self.push(
                node,
                FactKind::TransactionWrapper,
                format!("tx:{}", callee.text()),
                BTreeMap::new(),
                in_tx,
            );
            return true;
        }
        false
    }

    fn new_expr(&mut self, node: Node<'_>, in_tx: bool) {
        let Some(constructor) = node.child_by_field_name(field::CONSTRUCTOR) else {
            return;
        };
        let Some(callee) = callee_of(constructor, self.src) else {
            return;
        };
        let count = argc(node.child_by_field_name(field::ARGUMENTS));
        let mut detail = BTreeMap::new();
        if callee.name == "ValidationPipe" {
            detail.insert("validation".to_owned(), AttrValue::Bool(true));
        }
        self.push(node, FactKind::New, new_key(&callee, count), detail, in_tx);
    }

    fn condition(&mut self, node: Node<'_>, label: &str, in_tx: bool) {
        let Some(condition) = node.child_by_field_name(field::CONDITION) else {
            return;
        };
        let inner = unwrap_parens(condition);
        let mut detail = BTreeMap::new();
        detail.insert(
            "has_else".to_owned(),
            AttrValue::Bool(node.child_by_field_name(field::ALTERNATIVE).is_some()),
        );
        if label == "if" {
            let early = node
                .child_by_field_name(field::CONSEQUENCE)
                .is_some_and(is_early_exit);
            detail.insert("early_exit".to_owned(), AttrValue::Bool(early));
        }
        detail.insert(
            "compares_null".to_owned(),
            AttrValue::Bool(compares_null(inner, self.src)),
        );
        detail.insert(
            "negated".to_owned(),
            AttrValue::Bool(
                inner.kind() == kind::UNARY_EXPRESSION && self.src.text(inner).starts_with('!'),
            ),
        );
        detail.insert(
            "idents".to_owned(),
            AttrValue::List(
                idents(inner, self.src)
                    .into_iter()
                    .map(AttrValue::Str)
                    .collect(),
            ),
        );
        let key = format!("{label}:{}", h8(inner, self.src));
        self.push(node, FactKind::Condition, key, detail, in_tx);
    }

    fn switch(&mut self, node: Node<'_>, in_tx: bool) {
        let Some(value) = node.child_by_field_name(field::VALUE) else {
            return;
        };
        let inner = unwrap_parens(value);
        let mut detail = BTreeMap::new();
        let cases = node
            .child_by_field_name(field::BODY)
            .map(|body| {
                let mut cursor = body.walk();
                let cases = body
                    .named_children(&mut cursor)
                    .filter(|c| c.kind() == kind::SWITCH_CASE || c.kind() == kind::SWITCH_DEFAULT)
                    .count();
                cases
            })
            .unwrap_or(0);
        detail.insert("cases".to_owned(), AttrValue::Int(cases as i64));
        detail.insert(
            "idents".to_owned(),
            AttrValue::List(
                idents(inner, self.src)
                    .into_iter()
                    .map(AttrValue::Str)
                    .collect(),
            ),
        );
        let key = format!("switch:{}", h8(inner, self.src));
        self.push(node, FactKind::Condition, key, detail, in_tx);
    }

    fn loop_stmt(&mut self, node: Node<'_>, in_tx: bool) {
        let (label, header) = match node.kind() {
            kind::FOR_STATEMENT => ("for", self.header_text(node)),
            kind::WHILE_STATEMENT => (
                "while",
                node.child_by_field_name(field::CONDITION)
                    .map(|c| self.src.text(c).into_owned())
                    .unwrap_or_default(),
            ),
            _ => (
                "do",
                node.child_by_field_name(field::CONDITION)
                    .map(|c| self.src.text(c).into_owned())
                    .unwrap_or_default(),
            ),
        };
        self.push_loop(node, label, &header, in_tx);
    }

    fn for_in(&mut self, node: Node<'_>, in_tx: bool) {
        let mut is_of = false;
        let mut is_await = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "of" => is_of = true,
                "await" => is_await = true,
                _ => {}
            }
        }
        let header = self.header_text(node);
        self.push_loop(
            node,
            if is_of { "for_of" } else { "for_in" },
            &header,
            in_tx,
        );
        if is_await {
            self.push(
                node,
                FactKind::Await,
                "await:for_of".to_owned(),
                BTreeMap::new(),
                in_tx,
            );
        }
    }

    /// The loop header: the text from the statement start to its body.
    fn header_text(&self, node: Node<'_>) -> String {
        let end = node
            .child_by_field_name(field::BODY)
            .map(|body| body.start_byte())
            .unwrap_or_else(|| node.end_byte());
        self.src.slice(node.start_byte(), end).into_owned()
    }

    fn push_loop(&mut self, node: Node<'_>, label: &str, header: &str, in_tx: bool) {
        let mut detail = BTreeMap::new();
        let awaits = node
            .child_by_field_name(field::BODY)
            .is_some_and(|body| contains_kind(body, kind::AWAIT_EXPRESSION));
        detail.insert("awaits_inside".to_owned(), AttrValue::Bool(awaits));
        let key = format!("loop:{label}:{}", h8_text(header));
        self.push(node, FactKind::Loop, key, detail, in_tx);
    }

    fn throw(&mut self, node: Node<'_>, in_tx: bool) {
        let thrown = node.named_child(0).filter(|c| c.kind() != kind::COMMENT);
        let key = match thrown {
            Some(expr) if expr.kind() == kind::NEW_EXPRESSION => expr
                .child_by_field_name(field::CONSTRUCTOR)
                .and_then(|c| callee_of(c, self.src))
                .map(|callee| format!("throw:{}", callee.name))
                .unwrap_or_else(|| "throw:expr".to_owned()),
            _ => "throw:expr".to_owned(),
        };
        self.push(node, FactKind::Throw, key, BTreeMap::new(), in_tx);
    }

    fn try_catch(&mut self, node: Node<'_>, in_tx: bool) {
        let handler = node.child_by_field_name(field::HANDLER);
        let finalizer = node.child_by_field_name(field::FINALIZER);
        let mut detail = BTreeMap::new();
        if let Some(handler) = handler {
            let body = handler.child_by_field_name(field::BODY);
            let empty = body.is_none_or(|b| {
                let mut cursor = b.walk();
                let all_comments = b
                    .named_children(&mut cursor)
                    .all(|c| c.kind() == kind::COMMENT);
                all_comments
            });
            detail.insert("empty_catch".to_owned(), AttrValue::Bool(empty));
            detail.insert(
                "rethrows".to_owned(),
                AttrValue::Bool(body.is_some_and(|b| contains_kind(b, kind::THROW_STATEMENT))),
            );
            if let Some(param) = handler.child_by_field_name(field::PARAMETER) {
                detail.insert(
                    "catch_param".to_owned(),
                    AttrValue::Str(self.src.text(param).into_owned()),
                );
            }
        }
        let key = format!(
            "try:{}:{}",
            if handler.is_some() {
                "catch"
            } else {
                "nocatch"
            },
            if finalizer.is_some() {
                "finally"
            } else {
                "nofinally"
            }
        );
        self.push(node, FactKind::TryCatch, key, detail, in_tx);
    }

    fn await_expr(&mut self, node: Node<'_>, in_tx: bool) {
        let inner = node.named_child(0).map(unwrap_parens);
        let key = match inner {
            Some(call) if call.kind() == kind::CALL_EXPRESSION => call
                .child_by_field_name(field::FUNCTION)
                .and_then(|f| callee_of(f, self.src))
                .map(|callee| format!("await:{}", callee.text()))
                .unwrap_or_else(|| "await:expr".to_owned()),
            _ => "await:expr".to_owned(),
        };
        self.push(node, FactKind::Await, key, BTreeMap::new(), in_tx);
    }

    fn return_stmt(&mut self, node: Node<'_>, in_tx: bool) {
        let value = {
            let mut cursor = node.walk();
            let found = node
                .named_children(&mut cursor)
                .find(|c| c.kind() != kind::COMMENT);
            found
        };
        let shape = match value.map(unwrap_parens) {
            None => "void".to_owned(),
            Some(v) => match v.kind() {
                kind::NULL => "null".to_owned(),
                kind::UNDEFINED => "undefined".to_owned(),
                kind::TRUE => "true".to_owned(),
                kind::FALSE => "false".to_owned(),
                kind::NUMBER | kind::STRING | kind::TEMPLATE_STRING => "lit".to_owned(),
                kind::IDENTIFIER if self.src.text(v) == "undefined" => "undefined".to_owned(),
                kind::IDENTIFIER | kind::THIS => "ident".to_owned(),
                kind::OBJECT => "obj".to_owned(),
                kind::CALL_EXPRESSION => v
                    .child_by_field_name(field::FUNCTION)
                    .and_then(|f| callee_of(f, self.src))
                    .map(|callee| format!("call:{}", callee.text()))
                    .unwrap_or_else(|| format!("expr:{}", h8(v, self.src))),
                _ => format!("expr:{}", h8(v, self.src)),
            },
        };
        self.push(
            node,
            FactKind::Return,
            format!("return:{shape}"),
            BTreeMap::new(),
            in_tx,
        );
    }

    fn assignment(&mut self, node: Node<'_>, in_tx: bool) {
        let Some(left) = node.child_by_field_name(field::LEFT) else {
            return;
        };
        let left = unwrap_parens(left);
        if left.kind() != kind::MEMBER_EXPRESSION && left.kind() != kind::SUBSCRIPT_EXPRESSION {
            return;
        }
        let segments = crate::visit::fact_keys::chain_segments(left, self.src, 0);
        if segments.is_empty() {
            return;
        }
        let mut detail = BTreeMap::new();
        if node.kind() == kind::AUGMENTED_ASSIGNMENT_EXPRESSION {
            detail.insert("augmented".to_owned(), AttrValue::Bool(true));
        }
        self.push(
            node,
            FactKind::Assignment,
            format!("assign:{}", segments.join(".")),
            detail,
            in_tx,
        );
    }

    /// `process.env.NAME` and `process.env['NAME']`.
    fn env_read(&mut self, node: Node<'_>, in_tx: bool) {
        let Some(object) = node.child_by_field_name(field::OBJECT) else {
            return;
        };
        if !is_process_env(object, self.src) {
            return;
        }
        let name = if node.kind() == kind::MEMBER_EXPRESSION {
            node.child_by_field_name(field::PROPERTY)
                .map(|p| self.src.text(p).into_owned())
        } else {
            node.child_by_field_name(field::INDEX)
                .filter(|index| index.kind() == kind::STRING)
                .map(|index| crate::visit::expr::string_content(index, self.src))
        };
        if let Some(name) = name.filter(|n| is_env_name(n)) {
            self.push_env(node, &name, in_tx);
        }
    }

    /// `const { NAME, OTHER: alias } = process.env`.
    fn env_destructure(&mut self, node: Node<'_>, in_tx: bool) {
        let (Some(name), Some(value)) = (
            node.child_by_field_name(field::NAME),
            node.child_by_field_name(field::VALUE),
        ) else {
            return;
        };
        if name.kind() != kind::OBJECT_PATTERN || !is_process_env(unwrap_parens(value), self.src) {
            return;
        }
        let mut cursor = name.walk();
        let entries: Vec<Node<'_>> = name.named_children(&mut cursor).collect();
        for entry in entries {
            let key = match entry.kind() {
                kind::SHORTHAND_PROPERTY_IDENTIFIER_PATTERN => {
                    Some(self.src.text(entry).into_owned())
                }
                kind::PAIR_PATTERN => entry
                    .child_by_field_name(field::KEY)
                    .map(|k| crate::visit::expr::key_text(k, self.src)),
                kind::OBJECT_ASSIGNMENT_PATTERN => entry
                    .child_by_field_name(field::LEFT)
                    .map(|k| self.src.text(k).into_owned()),
                _ => None,
            };
            if let Some(key) = key.filter(|n| is_env_name(n)) {
                self.push_env(entry, &key, in_tx);
            }
        }
    }

    fn push_env(&mut self, node: Node<'_>, name: &str, in_tx: bool) {
        let mut detail = BTreeMap::new();
        detail.insert("name".to_owned(), AttrValue::Str(name.to_owned()));
        self.push(
            node,
            FactKind::ConfigRead,
            format!("env:{name}"),
            detail,
            in_tx,
        );
    }
}

fn args_text(args: &[IrExpr]) -> String {
    format!("{args:?}")
}

fn is_env_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_process_env(node: Node<'_>, src: &Source<'_>) -> bool {
    node.kind() == kind::MEMBER_EXPRESSION
        && crate::visit::expr::member_chain(node, src)
            .is_some_and(|chain| chain.len() == 2 && chain[0] == "process" && chain[1] == "env")
}

fn unwrap_parens(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    for _ in 0..16 {
        if current.kind() != kind::PARENTHESIZED_EXPRESSION {
            break;
        }
        match current.named_child(0) {
            Some(inner) => current = inner,
            None => break,
        }
    }
    current
}

fn first_argument(arguments: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = arguments.walk();
    let found = arguments
        .named_children(&mut cursor)
        .find(|c| c.kind() != kind::COMMENT);
    found
}

/// A guard clause: the consequence is a `return`/`throw`, alone or as the only statement of a
/// block.
fn is_early_exit(consequence: Node<'_>) -> bool {
    let exits =
        |n: Node<'_>| n.kind() == kind::RETURN_STATEMENT || n.kind() == kind::THROW_STATEMENT;
    if exits(consequence) {
        return true;
    }
    if consequence.kind() == kind::STATEMENT_BLOCK {
        let mut cursor = consequence.walk();
        let statements: Vec<Node<'_>> = consequence
            .named_children(&mut cursor)
            .filter(|c| c.kind() != kind::COMMENT)
            .collect();
        return statements.len() == 1 && statements.first().is_some_and(|s| exits(*s));
    }
    false
}

/// Whether a subtree contains a node of `wanted` kind (bounded).
fn contains_kind(node: Node<'_>, wanted: &str) -> bool {
    let mut stack = vec![node];
    let mut seen = 0usize;
    while let Some(current) = stack.pop() {
        seen += 1;
        if seen > MAX_SCAN_NODES {
            return false;
        }
        if current.kind() == wanted {
            return true;
        }
        let mut cursor = current.walk();
        stack.extend(current.children(&mut cursor));
    }
    false
}

/// Whether a condition mentions `null` or `undefined`.
fn compares_null(node: Node<'_>, src: &Source<'_>) -> bool {
    let mut stack = vec![node];
    let mut seen = 0usize;
    while let Some(current) = stack.pop() {
        seen += 1;
        if seen > MAX_SCAN_NODES {
            return false;
        }
        match current.kind() {
            kind::NULL | kind::UNDEFINED => return true,
            kind::IDENTIFIER if src.text(current) == "undefined" => return true,
            _ => {}
        }
        let mut cursor = current.walk();
        stack.extend(current.children(&mut cursor));
    }
    false
}

/// Up to [`MAX_CONDITION_IDENTS`] distinct identifiers of a condition, sorted.
fn idents(node: Node<'_>, src: &Source<'_>) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![node];
    let mut seen = 0usize;
    while let Some(current) = stack.pop() {
        seen += 1;
        if seen > MAX_SCAN_NODES {
            break;
        }
        if matches!(
            current.kind(),
            kind::IDENTIFIER | kind::PROPERTY_IDENTIFIER | kind::SHORTHAND_PROPERTY_IDENTIFIER
        ) {
            out.insert(src.text(current).into_owned());
        }
        let mut cursor = current.walk();
        stack.extend(current.children(&mut cursor));
    }
    out.into_iter().take(MAX_CONDITION_IDENTS).collect()
}

fn is_validation(callee: &Callee) -> bool {
    let name = callee.name.as_str();
    if name.starts_with("validate") || name.starts_with("assert") || name == "plainToInstance" {
        return true;
    }
    if name == "parse" || name == "safeParse" {
        let zod_like = callee.receiver.first().is_some_and(|first| first == "z")
            || callee
                .receiver_name()
                .is_some_and(|r| r.to_ascii_lowercase().ends_with("schema"));
        return zod_like;
    }
    false
}

fn is_transaction(callee: &Callee) -> bool {
    matches!(
        callee.name.as_str(),
        "transaction"
            | "runInTransaction"
            | "startTransaction"
            | "commitTransaction"
            | "rollbackTransaction"
    )
}
