//! Jest adapter: suites, cases and mock references (NEST-007). Test callbacks are not symbols,
//! so everything folds into the enclosing symbol, usually the module symbol.

mod mocks;

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, FrameworkFactKind, IrSymbol, LocalId};
use review_core::location::SourceRange;
use review_core::symbol::SymbolKind;
use tree_sitter::Node;

use super::util::{arguments_of, expr_text, node_string, s, str_list};
use super::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::expr::{member_chain, to_ir_expr};
use crate::visit::MAX_VISIT_DEPTH;

pub const ADAPTER: &str = "jest";

const JEST_GLOBALS: &str = "@jest/globals";

/// Separator of suite path components, the same one the graph uses in test node ids.
pub const SEPARATOR: &str = " \u{203a} ";

/// `*.spec.*`, `*.test.*`, `*.e2e-spec.*`, `*.e2e.*`, `__tests__/` or a top-level `test/` or
/// `tests/` directory.
pub fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or("");
    name.contains(".spec.")
        || name.contains(".test.")
        || name.contains(".e2e-spec.")
        || name.contains(".e2e.")
        || lower.split('/').any(|seg| seg == "__tests__")
        || lower.starts_with("test/")
        || lower.starts_with("tests/")
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

/// A test path, a `@jest/globals` import, or any `describe`/`it`/`test`/`jest` mention.
pub fn detect(ctx: &FrameworkCtx<'_>) -> bool {
    let needles: [&[u8]; 6] = [b"describe", b"it(", b"it.", b"test(", b"test.", b"jest."];
    is_test_path(ctx.path)
        || ctx.imports_from(JEST_GLOBALS)
        || needles.iter().any(|needle| contains(ctx.src.bytes, needle))
}

pub fn run(ctx: &mut FrameworkCtx<'_>) {
    let test_file = is_test_path(ctx.path);
    let root = ctx.root;
    let mut walker = Walker {
        ctx,
        suite: Vec::new(),
        dup: BTreeMap::new(),
        index: BTreeMap::new(),
        test_file,
    };
    walker.walk(root, 0);
}

struct TestCall {
    is_suite: bool,
    modifier: &'static str,
    concurrent: bool,
    each: bool,
}

struct Walker<'c, 'a> {
    ctx: &'c mut FrameworkCtx<'a>,
    suite: Vec<String>,
    dup: BTreeMap<String, u32>,
    index: BTreeMap<String, u32>,
    test_file: bool,
}

/// Whether `name` is declared at module level or as a parameter of an enclosing symbol, which
/// shadows the Jest global.
fn shadowed(ctx: &FrameworkCtx<'_>, name: &str, call: &SourceRange) -> bool {
    if let Some(b) = ctx.binding(name) {
        if b.specifier != JEST_GLOBALS {
            return true;
        }
        return false;
    }
    ctx.symbols.iter().any(|sym: &IrSymbol| {
        let declares = sym.name == name
            && sym.parent == Some(LocalId(0))
            && matches!(
                sym.kind,
                SymbolKind::Function
                    | SymbolKind::Variable
                    | SymbolKind::Constant
                    | SymbolKind::Class
                    | SymbolKind::Namespace
                    | SymbolKind::Enum
            );
        let param = sym.range.start <= call.start
            && call.start <= sym.range.end
            && sym.params.iter().any(|p| p.name == name);
        declares || param
    })
}

fn range_attr(range: SourceRange) -> AttrValue {
    AttrValue::List(vec![
        AttrValue::Int(i64::from(range.start.line)),
        AttrValue::Int(i64::from(range.start.column)),
        AttrValue::Int(i64::from(range.end.line)),
        AttrValue::Int(i64::from(range.end.column)),
    ])
}

/// `${...}` substitutions become the `${…}` placeholder.
fn placeholder(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
            let mut depth = 0i32;
            while i < chars.len() {
                if chars[i] == '{' {
                    depth += 1;
                } else if chars[i] == '}' {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            out.push_str("${\u{2026}}");
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

impl Walker<'_, '_> {
    fn walk(&mut self, node: Node<'_>, depth: usize) {
        if depth > MAX_VISIT_DEPTH {
            return;
        }
        let mut pushed = false;
        if node.kind() == kind::CALL_EXPRESSION {
            match self.classify(node) {
                Some(call) => pushed = self.emit_test(node, &call),
                None => mocks::visit(self.ctx, node),
            }
        }
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        for child in children {
            self.walk(child, depth + 1);
        }
        if pushed {
            self.suite.pop();
        }
    }

    fn classify(&self, call: Node<'_>) -> Option<TestCall> {
        let function = call.child_by_field_name(field::FUNCTION)?;
        let table_call = function.kind() == kind::CALL_EXPRESSION;
        let inner = if table_call {
            function.child_by_field_name(field::FUNCTION)?
        } else {
            function
        };
        let chain = member_chain(inner, self.ctx.src)?;
        let (base, modifiers) = chain.split_first()?;
        let (is_suite, mut modifier) = match base.as_str() {
            "describe" => (true, "none"),
            "xdescribe" => (true, "skip"),
            "fdescribe" => (true, "only"),
            "it" | "test" => (false, "none"),
            "xit" | "xtest" => (false, "skip"),
            "fit" => (false, "only"),
            _ => return None,
        };
        let mut each = false;
        let mut concurrent = false;
        for m in modifiers {
            match m.as_str() {
                "skip" => modifier = "skip",
                "only" => modifier = "only",
                "todo" => modifier = "todo",
                "each" => each = true,
                "concurrent" => concurrent = true,
                "failing" | "sequential" => {}
                _ => return None,
            }
        }
        // `describe.each(table)` alone is the table, not a suite; `describe.each(table)(...)`
        // is the suite.
        if each != table_call {
            return None;
        }
        if shadowed(self.ctx, base, &self.ctx.src.range(call)) {
            return None;
        }
        if each && modifier == "none" {
            modifier = "each";
        }
        Some(TestCall {
            is_suite,
            modifier,
            concurrent,
            each,
        })
    }

    /// Emits the suite or case fact. Returns true when a suite name was pushed.
    fn emit_test(&mut self, node: Node<'_>, call: &TestCall) -> bool {
        let src = self.ctx.src;
        let args = arguments_of(node);
        let (raw_name, dynamic) = match args.first() {
            Some(arg) => match node_string(*arg, src) {
                Some(text) if text.contains("${") => (placeholder(&text), true),
                Some(text) => (text, false),
                None => (expr_text(&to_ir_expr(*arg, src)), true),
            },
            None => ("{unknown}".to_owned(), true),
        };
        let parent_path = self.suite.join(SEPARATOR);
        let dup_key = format!("{parent_path}|{}|{raw_name}", call.is_suite);
        let seen = self.dup.entry(dup_key).or_insert(0);
        *seen += 1;
        let name = if *seen > 1 {
            format!("{raw_name} #{seen}")
        } else {
            raw_name
        };
        let position = self.index.entry(parent_path.clone()).or_insert(0);
        let index = *position;
        *position += 1;

        let range = src.range(node);
        let owner = self.ctx.owner(range.start);
        let mut attrs = BTreeMap::new();
        attrs.insert("symbol".to_owned(), AttrValue::Str(self.ctx.qname(owner)));
        attrs.insert("suite_path".to_owned(), AttrValue::Str(parent_path));
        attrs.insert("suite_path_parts".to_owned(), str_list(self.suite.clone()));
        attrs.insert("name".to_owned(), AttrValue::Str(name.clone()));
        attrs.insert("name_dynamic".to_owned(), AttrValue::Bool(dynamic));
        attrs.insert("modifier".to_owned(), s(call.modifier));
        attrs.insert("each".to_owned(), AttrValue::Bool(call.each));
        attrs.insert("range".to_owned(), range_attr(range));
        attrs.insert("is_test_file".to_owned(), AttrValue::Bool(self.test_file));
        if !call.is_suite {
            attrs.insert("index".to_owned(), AttrValue::Int(i64::from(index)));
            attrs.insert("concurrent".to_owned(), AttrValue::Bool(call.concurrent));
        }
        let base_conf: f32 = if self.test_file { 0.95 } else { 0.7 };
        let confidence = if dynamic {
            base_conf.min(0.5)
        } else {
            base_conf
        };
        self.ctx.emit(
            ADAPTER,
            if call.is_suite {
                FrameworkFactKind::TestSuite
            } else {
                FrameworkFactKind::TestCase
            },
            Some(owner),
            attrs,
            range,
            confidence,
        );
        if call.is_suite {
            self.suite.push(name);
            return true;
        }
        false
    }
}
