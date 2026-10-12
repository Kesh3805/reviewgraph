//! One walk over the tree that produces both the per-symbol syntax facts and the references.
//!
//! Every node is attributed to the innermost enclosing symbol by its span. Nested functions,
//! lambdas and comprehensions are not symbols, so what they do folds into the enclosing symbol;
//! module level statements land on the module symbol (`LocalId(0)`). Facts are emitted in source
//! order and groups are sorted by `LocalId`.

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::symbol::FloatBits;
use analysis_ir::{
    AnalyzerConfig, AttrValue, DiagCode, DiagSeverity, FactKind, IrReference, IrSymbol, LocalId,
    RefKind, SymbolFacts, SyntaxFact,
};
use review_core::symbol::{Hash128, SymbolKind};
use tree_sitter::Node;

use crate::diagnostics::DiagnosticSink;
use crate::exprs::{children_of, collapse_ws, dotted, string_value};
use crate::hashing::node_tokens;
use crate::imports::BindingTable;
use crate::refs::{bare, chain_parts, RefCtx, DYNAMIC_CALLS};
use crate::symbols::{Table, MAX_VISIT_DEPTH};
use crate::text::Source;

/// Facts kept per symbol; beyond this one info diagnostic is emitted and the rest are dropped.
pub const MAX_FACTS_PER_SYMBOL: usize = 2_000;

/// Identifiers kept in a condition's `idents` detail.
const MAX_CONDITION_IDENTS: usize = 8;

/// Bound for the small sub-walks (await inside a loop, identifiers of a condition).
const MAX_SCAN_NODES: usize = 4_096;

/// Receiver names (substrings, lowercase) that make a call database-like.
const DB_RECEIVERS: &[&str] = &[
    "session",
    "db",
    "repo",
    "repository",
    "manager",
    "objects",
    "cursor",
    "conn",
    "collection",
    "orm",
    "queryset",
    "engine",
];

const DB_WRITE_METHODS: &[&str] = &[
    "add",
    "add_all",
    "delete",
    "merge",
    "bulk_save_objects",
    "bulk_insert_mappings",
    "bulk_update_mappings",
    "save",
    "create",
    "update",
    "insert",
    "insert_one",
    "insert_many",
    "update_one",
    "update_many",
    "delete_one",
    "delete_many",
    "replace_one",
    "bulk_create",
    "bulk_update",
    "get_or_create",
    "update_or_create",
];

const DB_READ_METHODS: &[&str] = &[
    "query",
    "get",
    "filter",
    "filter_by",
    "all",
    "first",
    "one",
    "one_or_none",
    "scalars",
    "scalar",
    "find",
    "find_one",
    "count",
    "exists",
    "fetchall",
    "fetchone",
    "fetchmany",
];

const SQL_METHODS: &[&str] = &["execute", "executemany", "exec_driver_sql"];

/// Last segments of callables that open a transaction.
const TX_CALLEES: &[&str] = &["begin", "begin_nested", "atomic", "transaction"];

/// Default guard decorator fragments (lowercase substrings of the last name segment).
const GUARD_FRAGMENTS: &[&str] = &[
    "login_required",
    "permission",
    "auth",
    "require_role",
    "roles_",
    "superuser",
    "staff_member",
];

/// What the walk produced.
#[derive(Debug, Default)]
pub struct Walked {
    pub facts: Vec<SymbolFacts>,
    pub references: Vec<IrReference>,
}

struct Walker<'a> {
    refs: RefCtx<'a>,
    spans: BTreeMap<(usize, usize), u32>,
    cfg: &'a AnalyzerConfig,
    sink: &'a mut DiagnosticSink,
    facts: BTreeMap<u32, Vec<SyntaxFact>>,
    references: Vec<IrReference>,
    test_file: bool,
    capped: bool,
}

/// Runs the walk over `root`.
pub fn collect<'a>(
    root: Node<'_>,
    src: Source<'a>,
    table: &'a Table<'_>,
    bindings: &'a BindingTable,
    cfg: &'a AnalyzerConfig,
    test_file: bool,
    sink: &'a mut DiagnosticSink,
) -> Walked {
    let mut spans = BTreeMap::new();
    for (index, entry) in table.entries.iter().enumerate() {
        spans.insert(
            (entry.outer.start_byte(), entry.outer.end_byte()),
            u32::try_from(index).unwrap_or(u32::MAX),
        );
    }
    let mut walker = Walker {
        refs: RefCtx {
            src,
            bindings,
            symbols: &table.symbols,
        },
        spans,
        cfg,
        sink,
        facts: BTreeMap::new(),
        references: Vec::new(),
        test_file,
        capped: false,
    };
    walker.walk(root, 0, false, 0);
    if walker.capped {
        walker.sink.note(
            DiagSeverity::Info,
            DiagCode::UnsupportedConstruct,
            "fact limit reached for a symbol; further facts dropped",
            None,
        );
    }
    Walked {
        facts: walker
            .facts
            .into_iter()
            .map(|(symbol, facts)| SymbolFacts {
                symbol: LocalId(symbol),
                facts,
            })
            .collect(),
        references: walker.references,
    }
}

fn all_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// Whether any node of one of `kinds` lies inside `node` (bounded).
fn contains_kind(node: Node<'_>, kinds: &[&str]) -> bool {
    let mut budget = MAX_SCAN_NODES;
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if budget == 0 {
            return false;
        }
        budget -= 1;
        if kinds.contains(&current.kind()) {
            return true;
        }
        stack.extend(all_children(current));
    }
    false
}

fn argument_count(call: Node<'_>) -> usize {
    match call.child_by_field_name("arguments") {
        Some(arguments) if arguments.kind() == "argument_list" => children_of(arguments).len(),
        Some(_) => 1,
        None => 0,
    }
}

fn first_argument(call: Node<'_>) -> Option<Node<'_>> {
    let arguments = call.child_by_field_name("arguments")?;
    if arguments.kind() != "argument_list" {
        return None;
    }
    children_of(arguments).into_iter().next()
}

impl Walker<'_> {
    fn src(&self) -> Source<'_> {
        self.refs.src
    }

    fn in_test(&self, from: u32) -> bool {
        if self.test_file {
            return true;
        }
        let mut cursor = self.refs.symbols.get(from as usize);
        while let Some(symbol) = cursor {
            if symbol.name.starts_with("test_") || symbol.name.starts_with("Test") {
                return true;
            }
            cursor = symbol
                .parent
                .and_then(|p| self.refs.symbols.get(p.0 as usize));
        }
        false
    }

    fn push_fact(
        &mut self,
        symbol: u32,
        kind: FactKind,
        key: String,
        node: Node<'_>,
        mut detail: BTreeMap<String, AttrValue>,
        in_tx: bool,
    ) {
        if !self.cfg.syntax_facts {
            return;
        }
        let range = self.src().range(node);
        let entry = self.facts.entry(symbol).or_default();
        if entry.len() >= MAX_FACTS_PER_SYMBOL {
            self.capped = true;
            return;
        }
        if in_tx {
            detail.insert("in_transaction".to_owned(), AttrValue::Bool(true));
        }
        entry.push(SyntaxFact {
            kind,
            key,
            range,
            detail,
        });
    }

    /// First 8 hex characters of a BLAKE3-based hash over the normalized tokens of `nodes`.
    fn h8(&self, nodes: &[Node<'_>]) -> String {
        let mut text = String::new();
        for node in nodes {
            for token in node_tokens(*node, &self.src(), &[]) {
                text.push_str(&format!("{}{}\u{1f}", token.class.tag(), token.text));
            }
        }
        let hash = Hash128::of("rg.fact.v1", text.as_bytes());
        let bytes = hash.as_bytes();
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        )
    }

    fn walk(&mut self, node: Node<'_>, current: u32, in_tx: bool, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            return;
        }
        let mut current = current;
        let mut in_tx = in_tx;
        if let Some(id) = self
            .spans
            .get(&(node.start_byte(), node.end_byte()))
            .copied()
        {
            if id != current {
                current = id;
                in_tx = false;
            }
        }
        match node.kind() {
            "call" => self.on_call(node, current, in_tx),
            "class_definition" => self.on_class(node, current),
            "decorator" => self.on_decorator(node, current, in_tx),
            "if_statement" => self.on_if(node, current, in_tx, true),
            "elif_clause" => self.on_if(node, current, in_tx, false),
            "conditional_expression" => self.on_ternary(node, current, in_tx),
            "match_statement" => self.on_match(node, current, in_tx),
            "for_statement" | "while_statement" => self.on_loop(node, current, in_tx),
            "raise_statement" => self.on_raise(node, current, in_tx),
            "try_statement" => self.on_try(node, current, in_tx),
            "await" => self.on_await(node, current, in_tx),
            "return_statement" => self.on_return(node, current, in_tx),
            "assignment" | "augmented_assignment" => self.on_assign(node, current, in_tx),
            "subscript" => self.on_subscript(node, current, in_tx),
            "with_statement" => {
                if self.on_with(node, current, in_tx) {
                    in_tx = true;
                }
            }
            _ => {}
        }
        for child in all_children(node) {
            self.walk(child, current, in_tx, depth + 1);
        }
    }

    fn on_call(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        let argc = argument_count(node);
        let mut parts = Vec::new();
        chain_parts(function, &self.src(), &mut parts, 0);
        let chain = parts.join(".");
        let terminal = parts.last().map(|p| bare(p).to_owned()).unwrap_or_default();
        let key = format!(
            "call:{}/{}",
            if chain.is_empty() {
                "?"
            } else {
                chain.as_str()
            },
            argc
        );
        let mut detail = BTreeMap::new();
        let lower = terminal.to_ascii_lowercase();
        if lower.starts_with("validate")
            || lower.starts_with("assert")
            || lower == "model_validate"
            || lower == "parse_obj"
            || lower == "parse_raw"
        {
            detail.insert("validation".to_owned(), AttrValue::Bool(true));
        }
        self.push_fact(current, FactKind::Call, key, node, detail, in_tx);
        self.db_fact(node, &parts, current, in_tx);
        self.env_fact(node, &parts, current, in_tx);
        let in_test = self.in_test(current);
        if let Some(reference) = self.refs.call_ref(node, current, argc, in_test) {
            if reference.attrs.contains_key("dynamic") {
                let range = reference.range;
                debug_assert!(DYNAMIC_CALLS.contains(&reference.name.as_str()));
                self.sink.note(
                    DiagSeverity::Info,
                    DiagCode::UnsupportedConstruct,
                    "dynamic attribute or code access",
                    Some(range),
                );
            }
            self.references.push(reference);
        }
    }

    fn sql_text(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "string" => string_value(node, &self.src()),
            "call" => first_argument(node).and_then(|a| string_value(a, &self.src())),
            _ => None,
        }
    }

    fn db_fact(&mut self, node: Node<'_>, parts: &[String], current: u32, in_tx: bool) {
        let Some((last, receiver)) = parts.split_last() else {
            return;
        };
        if receiver.is_empty() {
            return;
        }
        let method = bare(last).to_owned();
        let receiver_lower: Vec<String> = receiver
            .iter()
            .map(|p| bare(p).to_ascii_lowercase())
            .collect();
        if !receiver_lower
            .iter()
            .any(|p| DB_RECEIVERS.iter().any(|h| p.contains(h)))
        {
            return;
        }
        let first = first_argument(node);
        let mut confidence = 0.75_f64;
        let kind = if SQL_METHODS.contains(&method.as_str()) {
            let sql = first.and_then(|a| self.sql_text(a));
            let keyword = sql
                .as_deref()
                .and_then(|s| s.split_whitespace().next())
                .map(str::to_ascii_uppercase);
            confidence = 0.6;
            match keyword.as_deref() {
                Some("INSERT" | "UPDATE" | "DELETE" | "ALTER" | "DROP" | "TRUNCATE") => {
                    FactKind::DbWriteLike
                }
                Some("SELECT") => FactKind::DbReadLike,
                _ => return,
            }
        } else if DB_WRITE_METHODS.contains(&method.as_str()) {
            FactKind::DbWriteLike
        } else if DB_READ_METHODS.contains(&method.as_str()) {
            FactKind::DbReadLike
        } else {
            return;
        };
        let mut entity: Option<String> = None;
        let django_style = receiver_lower.iter().any(|p| p == "objects");
        if django_style {
            if let Some(first_part) = receiver.first() {
                let name = bare(first_part);
                if name.chars().next().is_some_and(char::is_uppercase) {
                    entity = Some(name.to_owned());
                    confidence = 0.9;
                }
            }
        }
        if entity.is_none() {
            if let Some(arg) = first {
                if arg.kind() == "call" {
                    if let Some(name) = arg
                        .child_by_field_name("function")
                        .and_then(|f| dotted(f, &self.src()))
                        .and_then(|p| p.last().cloned())
                    {
                        if name.chars().next().is_some_and(char::is_uppercase) {
                            entity = Some(name);
                        }
                    }
                }
            }
        }
        let prefix = if kind == FactKind::DbWriteLike {
            "dbw"
        } else {
            "dbr"
        };
        let key = format!("{prefix}:{method}:{}", entity.as_deref().unwrap_or("?"));
        let mut detail = BTreeMap::new();
        detail.insert("method".to_owned(), AttrValue::Str(method));
        if let Some(entity) = entity {
            detail.insert("entity".to_owned(), AttrValue::Str(entity));
        }
        detail.insert(
            "confidence".to_owned(),
            AttrValue::Float(FloatBits::new(confidence)),
        );
        self.push_fact(current, kind, key, node, detail, in_tx);
    }

    fn env_fact(&mut self, node: Node<'_>, parts: &[String], current: u32, in_tx: bool) {
        let joined = parts.join(".");
        let is_env = matches!(
            joined.as_str(),
            "os.environ.get" | "os.getenv" | "environ.get" | "getenv"
        );
        if !is_env {
            return;
        }
        let Some(name) = first_argument(node).and_then(|a| string_value(a, &self.src())) else {
            return;
        };
        let mut detail = BTreeMap::new();
        detail.insert("name".to_owned(), AttrValue::Str(name.clone()));
        self.push_fact(
            current,
            FactKind::ConfigRead,
            format!("env:{name}"),
            node,
            detail,
            in_tx,
        );
    }

    fn on_subscript(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        let Some(path) = dotted(value, &self.src()) else {
            return;
        };
        let joined = path.join(".");
        if joined != "os.environ" && joined != "environ" {
            return;
        }
        let Some(name) = children_of(node)
            .into_iter()
            .find(|c| c.kind() == "string")
            .and_then(|s| string_value(s, &self.src()))
        else {
            return;
        };
        let mut detail = BTreeMap::new();
        detail.insert("name".to_owned(), AttrValue::Str(name.clone()));
        self.push_fact(
            current,
            FactKind::ConfigRead,
            format!("env:{name}"),
            node,
            detail,
            in_tx,
        );
    }

    fn on_class(&mut self, node: Node<'_>, current: u32) {
        let Some(bases) = node.child_by_field_name("superclasses") else {
            return;
        };
        for base in children_of(bases) {
            if base.kind() == "keyword_argument" {
                continue;
            }
            if let Some(reference) = self.refs.name_ref(base, RefKind::Extends, current, 0) {
                self.references.push(reference);
            }
        }
    }

    fn on_decorator(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let Some(inner) = children_of(node).into_iter().next() else {
            return;
        };
        let argc = if inner.kind() == "call" {
            argument_count(inner)
        } else {
            0
        };
        if let Some(reference) = self.refs.name_ref(inner, RefKind::Decorator, current, argc) {
            self.references.push(reference);
        }
        let target = if inner.kind() == "call" {
            inner.child_by_field_name("function")
        } else {
            Some(inner)
        };
        let Some(path) = target.and_then(|t| dotted(t, &self.src())) else {
            return;
        };
        let joined = path.join(".");
        let name = path.last().cloned().unwrap_or_default();
        if matches!(
            joined.as_str(),
            "transaction.atomic" | "atomic" | "transactional" | "db.transaction"
        ) {
            self.push_fact(
                current,
                FactKind::TransactionWrapper,
                "tx:@Transactional".to_owned(),
                node,
                BTreeMap::new(),
                in_tx,
            );
        }
        if self.is_guard(&name) {
            let args_hash = match inner.child_by_field_name("arguments") {
                Some(arguments) => self.h8(&[arguments]),
                None => self.h8(&[]),
            };
            let mut detail = BTreeMap::new();
            detail.insert("name".to_owned(), AttrValue::Str(name.clone()));
            self.push_fact(
                current,
                FactKind::GuardDecorator,
                format!("guard:{name}:{args_hash}"),
                node,
                detail,
                in_tx,
            );
        }
    }

    fn is_guard(&self, name: &str) -> bool {
        match &self.cfg.guard_decorator_names {
            Some(names) => names.iter().any(|n| n == name),
            None => {
                let lower = name.to_ascii_lowercase();
                GUARD_FRAGMENTS.iter().any(|f| lower.contains(f))
            }
        }
    }

    fn condition_detail(
        &self,
        condition: Node<'_>,
        has_else: bool,
        early_exit: bool,
    ) -> BTreeMap<String, AttrValue> {
        let mut detail = BTreeMap::new();
        detail.insert("has_else".to_owned(), AttrValue::Bool(has_else));
        detail.insert("early_exit".to_owned(), AttrValue::Bool(early_exit));
        detail.insert(
            "compares_null".to_owned(),
            AttrValue::Bool(self.compares_none(condition)),
        );
        detail.insert(
            "negated".to_owned(),
            AttrValue::Bool(condition.kind() == "not_operator"),
        );
        let mut idents = BTreeSet::new();
        let mut budget = MAX_SCAN_NODES;
        let mut stack = vec![condition];
        while let Some(current) = stack.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            if current.kind() == "identifier" {
                idents.insert(self.src().text(current).into_owned());
            }
            stack.extend(all_children(current));
        }
        detail.insert(
            "idents".to_owned(),
            AttrValue::List(
                idents
                    .into_iter()
                    .take(MAX_CONDITION_IDENTS)
                    .map(AttrValue::Str)
                    .collect(),
            ),
        );
        detail
    }

    fn compares_none(&self, condition: Node<'_>) -> bool {
        let mut budget = MAX_SCAN_NODES;
        let mut stack = vec![condition];
        while let Some(current) = stack.pop() {
            if budget == 0 {
                return false;
            }
            budget -= 1;
            if current.kind() == "comparison_operator"
                && all_children(current).iter().any(|c| c.kind() == "none")
            {
                return true;
            }
            stack.extend(all_children(current));
        }
        false
    }

    fn on_if(&mut self, node: Node<'_>, current: u32, in_tx: bool, is_if: bool) {
        let Some(condition) = node.child_by_field_name("condition") else {
            return;
        };
        let has_else = all_children(node).iter().any(|c| c.kind() == "else_clause");
        let early_exit = is_if
            && node
                .child_by_field_name("consequence")
                .map(children_of)
                .is_some_and(|stmts| {
                    stmts.len() == 1
                        && matches!(
                            stmts[0].kind(),
                            "return_statement"
                                | "raise_statement"
                                | "continue_statement"
                                | "break_statement"
                        )
                });
        let detail = self.condition_detail(condition, has_else, early_exit);
        let key = format!("if:{}", self.h8(&[condition]));
        self.push_fact(current, FactKind::Condition, key, node, detail, in_tx);
    }

    fn on_ternary(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let parts = children_of(node);
        let Some(condition) = parts.get(1).copied() else {
            return;
        };
        let detail = self.condition_detail(condition, true, false);
        let key = format!("ternary:{}", self.h8(&[condition]));
        self.push_fact(current, FactKind::Condition, key, node, detail, in_tx);
    }

    fn on_match(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let Some(subject) = node.child_by_field_name("subject") else {
            return;
        };
        let cases = node
            .child_by_field_name("body")
            .map(|b| {
                children_of(b)
                    .iter()
                    .filter(|c| c.kind() == "case_clause")
                    .count()
            })
            .unwrap_or(0);
        let mut detail = self.condition_detail(subject, false, false);
        detail.insert(
            "cases".to_owned(),
            AttrValue::Int(i64::try_from(cases).unwrap_or(i64::MAX)),
        );
        let key = format!("switch:{}", self.h8(&[subject]));
        self.push_fact(current, FactKind::Condition, key, node, detail, in_tx);
    }

    fn on_loop(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let is_for = node.kind() == "for_statement";
        let header: Vec<Node<'_>> = if is_for {
            ["left", "right"]
                .iter()
                .filter_map(|f| node.child_by_field_name(f))
                .collect()
        } else {
            node.child_by_field_name("condition").into_iter().collect()
        };
        let label = if is_for { "for" } else { "while" };
        let key = format!("loop:{label}:{}", self.h8(&header));
        let awaits = node
            .child_by_field_name("body")
            .is_some_and(|b| contains_kind(b, &["await"]));
        let mut detail = BTreeMap::new();
        detail.insert("awaits_inside".to_owned(), AttrValue::Bool(awaits));
        self.push_fact(current, FactKind::Loop, key, node, detail, in_tx);
        if is_for && all_children(node).iter().any(|c| c.kind() == "async") {
            self.push_fact(
                current,
                FactKind::Await,
                "await:async_for".to_owned(),
                node,
                BTreeMap::new(),
                in_tx,
            );
        }
    }

    fn on_raise(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let key = match children_of(node).first() {
            None => "throw:rethrow".to_owned(),
            Some(exception) => {
                let target = if exception.kind() == "call" {
                    exception.child_by_field_name("function")
                } else {
                    Some(*exception)
                };
                match target
                    .and_then(|t| dotted(t, &self.src()))
                    .and_then(|p| p.last().cloned())
                {
                    Some(name) => format!("throw:{name}"),
                    None => "throw:expr".to_owned(),
                }
            }
        };
        self.push_fact(current, FactKind::Throw, key, node, BTreeMap::new(), in_tx);
    }

    fn on_try(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let clauses = children_of(node);
        let handlers: Vec<&Node<'_>> = clauses
            .iter()
            .filter(|c| c.kind() == "except_clause" || c.kind() == "except_group_clause")
            .collect();
        let has_finally = clauses.iter().any(|c| c.kind() == "finally_clause");
        let key = format!(
            "try:{}:{}",
            if handlers.is_empty() {
                "nocatch"
            } else {
                "catch"
            },
            if has_finally { "finally" } else { "nofinally" }
        );
        let mut empty_catch = false;
        let mut rethrows = false;
        let mut catch_param: Option<String> = None;
        for handler in &handlers {
            if let Some(block) = children_of(**handler)
                .into_iter()
                .find(|c| c.kind() == "block")
            {
                let statements = children_of(block);
                if statements.len() == 1 && statements[0].kind() == "pass_statement" {
                    empty_catch = true;
                }
                if contains_kind(block, &["raise_statement"]) {
                    rethrows = true;
                }
            }
            if catch_param.is_none() {
                catch_param = handler
                    .child_by_field_name("alias")
                    .map(|a| collapse_ws(&self.src().text(a)));
            }
        }
        let mut detail = BTreeMap::new();
        detail.insert("empty_catch".to_owned(), AttrValue::Bool(empty_catch));
        detail.insert("rethrows".to_owned(), AttrValue::Bool(rethrows));
        if let Some(param) = catch_param {
            detail.insert("catch_param".to_owned(), AttrValue::Str(param));
        }
        self.push_fact(current, FactKind::TryCatch, key, node, detail, in_tx);
    }

    fn on_await(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let mut key = "await:expr".to_owned();
        if let Some(inner) = children_of(node).first() {
            if inner.kind() == "call" {
                if let Some(function) = inner.child_by_field_name("function") {
                    let mut parts = Vec::new();
                    chain_parts(function, &self.src(), &mut parts, 0);
                    if !parts.is_empty() {
                        key = format!("await:{}", parts.join("."));
                    }
                }
            }
        }
        self.push_fact(current, FactKind::Await, key, node, BTreeMap::new(), in_tx);
    }

    fn on_return(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let key = match children_of(node).first() {
            None => "return:void".to_owned(),
            Some(value) => match value.kind() {
                "none" => "return:null".to_owned(),
                "true" => "return:true".to_owned(),
                "false" => "return:false".to_owned(),
                "string" | "integer" | "float" => "return:lit".to_owned(),
                "identifier" => "return:ident".to_owned(),
                "dictionary" => "return:obj".to_owned(),
                "call" => {
                    let mut parts = Vec::new();
                    if let Some(function) = value.child_by_field_name("function") {
                        chain_parts(function, &self.src(), &mut parts, 0);
                    }
                    format!("return:call:{}", parts.join("."))
                }
                _ => format!("return:expr:{}", self.h8(&[*value])),
            },
        };
        self.push_fact(current, FactKind::Return, key, node, BTreeMap::new(), in_tx);
    }

    fn on_assign(&mut self, node: Node<'_>, current: u32, in_tx: bool) {
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        if left.kind() != "attribute" {
            return;
        }
        let Some(path) = dotted(left, &self.src()) else {
            return;
        };
        let mut detail = BTreeMap::new();
        detail.insert(
            "augmented".to_owned(),
            AttrValue::Bool(node.kind() == "augmented_assignment"),
        );
        self.push_fact(
            current,
            FactKind::Assignment,
            format!("assign:{}", path.join(".")),
            node,
            detail,
            in_tx,
        );
    }

    /// Emits a transaction fact for `with session.begin():` style statements. Returns whether the
    /// body runs inside a transaction.
    fn on_with(&mut self, node: Node<'_>, current: u32, in_tx: bool) -> bool {
        let mut opened = false;
        for clause in children_of(node) {
            if clause.kind() != "with_clause" {
                continue;
            }
            for item in children_of(clause) {
                let Some(value) = item.child_by_field_name("value") else {
                    continue;
                };
                if value.kind() != "call" {
                    continue;
                }
                let Some(function) = value.child_by_field_name("function") else {
                    continue;
                };
                let mut parts = Vec::new();
                chain_parts(function, &self.src(), &mut parts, 0);
                let last = parts.last().map(|p| bare(p).to_owned()).unwrap_or_default();
                if TX_CALLEES.contains(&last.as_str()) {
                    opened = true;
                    self.push_fact(
                        current,
                        FactKind::TransactionWrapper,
                        format!("tx:{}", parts.join(".")),
                        value,
                        BTreeMap::new(),
                        in_tx,
                    );
                }
            }
        }
        opened
    }
}

/// Whether `symbols` has a class named like a test case holder. Used by adapters.
pub fn is_test_symbol(symbol: &IrSymbol) -> bool {
    matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
        && symbol.name.starts_with("test")
}
