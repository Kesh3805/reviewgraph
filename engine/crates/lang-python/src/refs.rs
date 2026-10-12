//! References from symbols to names, with receiver hints for the linker.

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, BindingRef, IrReference, IrSymbol, LocalId, ReceiverHint, RefKind};
use review_core::symbol::SymbolKind;
use tree_sitter::Node;

use crate::exprs::{children_of, dotted};
use crate::imports::BindingTable;
use crate::text::Source;

/// Deepest chain the helpers follow.
const MAX_CHAIN_DEPTH: usize = 32;

/// Calls whose target is chosen at run time. They produce a diagnostic and a low-confidence edge.
pub const DYNAMIC_CALLS: &[&str] = &[
    "getattr",
    "setattr",
    "delattr",
    "eval",
    "exec",
    "__import__",
    "import_module",
];

/// The parts of a receiver chain: `self.repo.save()` gives `["self", "repo", "save"]`; an inner
/// call is written `name()`, a subscript `name[]`, and literals are dropped.
pub fn chain_parts(node: Node<'_>, src: &Source<'_>, out: &mut Vec<String>, depth: usize) {
    if depth > MAX_CHAIN_DEPTH {
        return;
    }
    match node.kind() {
        "identifier" => out.push(src.text(node).into_owned()),
        "attribute" => {
            if let Some(object) = node.child_by_field_name("object") {
                chain_parts(object, src, out, depth + 1);
            }
            if let Some(attribute) = node.child_by_field_name("attribute") {
                out.push(src.text(attribute).into_owned());
            }
        }
        "call" => {
            if let Some(function) = node.child_by_field_name("function") {
                chain_parts(function, src, out, depth + 1);
                if let Some(last) = out.last_mut() {
                    last.push_str("()");
                }
            }
        }
        "subscript" => {
            if let Some(value) = node.child_by_field_name("value") {
                chain_parts(value, src, out, depth + 1);
                if let Some(last) = out.last_mut() {
                    last.push_str("[]");
                }
            }
        }
        "parenthesized_expression" => {
            if let Some(inner) = children_of(node).first() {
                chain_parts(*inner, src, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// A chain part without its `()` or `[]` suffix.
pub fn bare(part: &str) -> &str {
    part.trim_end_matches("[]").trim_end_matches("()")
}

/// Context needed to describe a reference.
#[derive(Debug)]
pub struct RefCtx<'a> {
    pub src: Source<'a>,
    pub bindings: &'a BindingTable,
    pub symbols: &'a [IrSymbol],
}

impl RefCtx<'_> {
    fn binding_of(&self, name: &str) -> Option<BindingRef> {
        self.bindings.get(name).map(|b| b.reference)
    }

    /// The declared type of `self.<field>`, from the annotated class attributes of the class that
    /// encloses `from`.
    fn field_type(&self, from: u32, field: &str) -> Option<String> {
        let mut cursor = self.symbols.get(from as usize)?;
        let class = loop {
            if cursor.kind == SymbolKind::Class {
                break cursor.local_id;
            }
            cursor = self.symbols.get(cursor.parent?.0 as usize)?;
        };
        self.symbols
            .iter()
            .find(|s| s.parent == Some(class) && s.kind == SymbolKind::Field && s.name == field)
            .and_then(|s| s.declared_type.clone())
    }

    fn identifier_hint(&self, name: &str) -> ReceiverHint {
        if name == "self" || name == "cls" {
            return ReceiverHint::This;
        }
        match self.bindings.get(name) {
            Some(binding) if binding.imported == "*" => ReceiverHint::ImportedNamespace {
                binding: binding.reference,
            },
            _ => ReceiverHint::Identifier {
                name: name.to_owned(),
                declared_type: None,
            },
        }
    }

    /// What is known about the object a member is accessed on.
    pub fn receiver_hint(&self, object: Node<'_>, from: u32) -> ReceiverHint {
        match object.kind() {
            "identifier" => self.identifier_hint(&self.src.text(object)),
            "attribute" => match dotted(object, &self.src) {
                Some(parts) if parts.len() == 2 && (parts[0] == "self" || parts[0] == "cls") => {
                    ReceiverHint::ThisField {
                        declared_type: self.field_type(from, &parts[1]),
                        field: parts[1].clone(),
                    }
                }
                Some(parts) if parts.len() > 1 => ReceiverHint::Chain {
                    root: Box::new(self.identifier_hint(&parts[0])),
                    segments: parts[1..].to_vec(),
                },
                _ => ReceiverHint::Unknown,
            },
            "call" => {
                let function = object.child_by_field_name("function");
                match function {
                    Some(f) if f.kind() == "identifier" && self.src.text(f) == "super" => {
                        ReceiverHint::Super
                    }
                    Some(f) => {
                        let mut parts = Vec::new();
                        chain_parts(f, &self.src, &mut parts, 0);
                        ReceiverHint::CallResult {
                            callee: parts.join("."),
                        }
                    }
                    None => ReceiverHint::Unknown,
                }
            }
            "subscript" => ReceiverHint::Computed,
            _ => ReceiverHint::Unknown,
        }
    }

    /// The root identifier of an attribute chain.
    fn root_name(&self, node: Node<'_>) -> Option<String> {
        let mut current = node;
        for _ in 0..MAX_CHAIN_DEPTH {
            match current.kind() {
                "identifier" => return Some(self.src.text(current).into_owned()),
                "attribute" => current = current.child_by_field_name("object")?,
                _ => return None,
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        from: u32,
        kind: RefKind,
        name: String,
        receiver: ReceiverHint,
        root: Option<String>,
        node: Node<'_>,
        arg_count: usize,
        in_test_block: bool,
    ) -> IrReference {
        let import_binding = root.as_deref().and_then(|r| self.binding_of(r));
        IrReference {
            from: LocalId(from),
            kind,
            name,
            receiver,
            import_binding,
            range: self.src.range(node),
            arg_count: u16::try_from(arg_count).unwrap_or(u16::MAX),
            in_test_block,
            attrs: BTreeMap::new(),
        }
    }

    /// The reference a call makes, or `None` when the callee has no terminal name (`f()()`).
    pub fn call_ref(
        &self,
        call: Node<'_>,
        from: u32,
        arg_count: usize,
        in_test_block: bool,
    ) -> Option<IrReference> {
        let function = call.child_by_field_name("function")?;
        let (name, receiver, root) = match function.kind() {
            "identifier" => {
                let name = self.src.text(function).into_owned();
                (name.clone(), ReceiverHint::None, Some(name))
            }
            "attribute" => {
                let attribute = function.child_by_field_name("attribute")?;
                let object = function.child_by_field_name("object")?;
                (
                    self.src.text(attribute).into_owned(),
                    self.receiver_hint(object, from),
                    self.root_name(object),
                )
            }
            _ => return None,
        };
        let dynamic = DYNAMIC_CALLS.contains(&name.as_str());
        let constructor_like = name.chars().next().is_some_and(char::is_uppercase);
        let mut reference = self.build(
            from,
            RefKind::Call,
            name,
            receiver,
            root,
            call,
            arg_count,
            in_test_block,
        );
        if dynamic {
            reference
                .attrs
                .insert("name_ambiguous".to_owned(), AttrValue::Bool(true));
            reference
                .attrs
                .insert("dynamic".to_owned(), AttrValue::Bool(true));
        }
        if constructor_like {
            reference
                .attrs
                .insert("constructor_like".to_owned(), AttrValue::Bool(true));
        }
        Some(reference)
    }

    /// A reference to a base class or a decorator expression (`Base`, `models.Model`,
    /// `app.get(...)`).
    pub fn name_ref(
        &self,
        expression: Node<'_>,
        kind: RefKind,
        from: u32,
        arg_count: usize,
    ) -> Option<IrReference> {
        let target = if expression.kind() == "call" {
            expression.child_by_field_name("function")?
        } else {
            expression
        };
        let (name, receiver, root) = match target.kind() {
            "identifier" => {
                let name = self.src.text(target).into_owned();
                (name.clone(), ReceiverHint::None, Some(name))
            }
            "attribute" => {
                let attribute = target.child_by_field_name("attribute")?;
                let object = target.child_by_field_name("object")?;
                (
                    self.src.text(attribute).into_owned(),
                    self.receiver_hint(object, from),
                    self.root_name(object),
                )
            }
            _ => return None,
        };
        Some(self.build(
            from, kind, name, receiver, root, expression, arg_count, false,
        ))
    }
}
