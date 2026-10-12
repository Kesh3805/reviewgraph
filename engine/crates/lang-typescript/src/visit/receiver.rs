//! Receiver-hint classification (TSA-005): how a callee, constructor or type was reached, which
//! is what the linker needs to pick a target (`this_member`, `di_constructor`, `type_annotation`,
//! `import` in the confidence table).

use std::collections::BTreeMap;

use analysis_ir::reference::BindingRef;
use analysis_ir::{AttrValue, Imported, IrImport, ReceiverHint};
use review_core::location::SourceRange;
use tree_sitter::Node;

use super::expr::{collapse_ws, member_chain, truncate_chars};
use super::type_env::{type_args, type_head, TypeEnv};
use crate::bindings::BindingTable;
use crate::kinds::{field, kind};
use crate::text::Source;

/// Longest `CallResult` callee text kept.
const MAX_CALLEE_CHARS: usize = 120;

const TYPE_ASSERTION: &str = "type_assertion";
const OPTIONAL_CHAIN: &str = "optional_chain";
const STATIC_CANDIDATE: &str = "static_candidate";
const OPTIONAL: &str = "optional";

/// A classified callee.
#[derive(Debug)]
pub struct Callee {
    pub name: String,
    pub name_range: SourceRange,
    pub receiver: ReceiverHint,
    pub import_binding: Option<BindingRef>,
    pub attrs: BTreeMap<String, AttrValue>,
}

/// What is known about one receiver expression.
#[derive(Debug)]
pub struct Hint {
    pub receiver: ReceiverHint,
    pub binding: Option<BindingRef>,
    pub static_candidate: bool,
}

impl Hint {
    fn plain(receiver: ReceiverHint) -> Self {
        Self {
            receiver,
            binding: None,
            static_candidate: false,
        }
    }
}

/// Classifies expressions against the module's import bindings and the local type environment.
#[derive(Debug)]
pub struct Resolver<'s, 'a> {
    pub src: &'s Source<'a>,
    pub imports: &'s [IrImport],
    pub bindings: &'s BindingTable,
}

/// Peels parentheses, `!`, `as`, `satisfies` and `<T>x` assertions.
pub fn unwrap_expr(mut node: Node<'_>) -> Node<'_> {
    loop {
        let inner = match node.kind() {
            kind::PARENTHESIZED_EXPRESSION
            | kind::NON_NULL_EXPRESSION
            | kind::AS_EXPRESSION
            | kind::SATISFIES_EXPRESSION => node.named_child(0),
            TYPE_ASSERTION => node
                .named_child_count()
                .checked_sub(1)
                .and_then(|last| node.named_child(last)),
            _ => None,
        };
        match inner {
            Some(next) => node = next,
            None => return node,
        }
    }
}

/// Whether a `?.` token is a direct child of `node`.
pub fn has_optional_chain(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == OPTIONAL_CHAIN {
            return true;
        }
    }
    false
}

fn is_capitalized(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

impl Resolver<'_, '_> {
    /// The import binding behind `name`, unless a local declaration shadows it.
    pub fn binding_for(&self, env: &TypeEnv, name: &str) -> Option<BindingRef> {
        if env.is_local(name) {
            return None;
        }
        self.bindings.get(name)
    }

    fn imported_kind(&self, binding: BindingRef) -> Option<&Imported> {
        self.imports
            .get(binding.import as usize)
            .and_then(|import| import.bindings.get(binding.binding as usize))
            .map(|b| &b.imported)
    }

    /// The hint for a bare identifier used as a receiver.
    pub fn root_hint(&self, env: &TypeEnv, name: &str) -> Hint {
        if let Some(binding) = self.binding_for(env, name) {
            return match self.imported_kind(binding) {
                Some(Imported::Namespace | Imported::Default | Imported::CjsModule) => Hint {
                    receiver: ReceiverHint::ImportedNamespace { binding },
                    binding: Some(binding),
                    static_candidate: false,
                },
                _ => Hint {
                    receiver: ReceiverHint::Identifier {
                        name: name.to_owned(),
                        declared_type: None,
                    },
                    binding: Some(binding),
                    static_candidate: is_capitalized(name),
                },
            };
        }
        let local = env.is_local(name);
        Hint {
            receiver: ReceiverHint::Identifier {
                name: name.to_owned(),
                declared_type: env.declared_type(name),
            },
            binding: None,
            static_candidate: !local && is_capitalized(name),
        }
    }

    /// The hint for a dotted path (`ns.sub` in `ns.sub.Type`): the root's hint, wrapped in a chain
    /// when there are further segments.
    pub fn path_hint(&self, env: &TypeEnv, parts: &[String]) -> Hint {
        let Some((first, rest)) = parts.split_first() else {
            return Hint::plain(ReceiverHint::Unknown);
        };
        let root = self.root_hint(env, first);
        if rest.is_empty() {
            return root;
        }
        Hint {
            receiver: ReceiverHint::Chain {
                root: Box::new(root.receiver),
                segments: rest.to_vec(),
            },
            binding: root.binding,
            static_candidate: false,
        }
    }

    /// Code-only text of a callee, without literals: `a.b().c` stays `a.b().c`.
    fn callee_text(&self, node: Node<'_>) -> String {
        let node = unwrap_expr(node);
        let text = match node.kind() {
            kind::MEMBER_EXPRESSION => {
                let object = node
                    .child_by_field_name(field::OBJECT)
                    .map(|o| self.callee_text(o))
                    .unwrap_or_default();
                let property = node
                    .child_by_field_name(field::PROPERTY)
                    .map(|p| self.src.text(p).into_owned())
                    .unwrap_or_default();
                format!("{object}.{property}")
            }
            kind::CALL_EXPRESSION => {
                let function = node
                    .child_by_field_name(field::FUNCTION)
                    .map(|f| self.callee_text(f))
                    .unwrap_or_default();
                format!("{function}()")
            }
            kind::IDENTIFIER | kind::THIS | kind::SUPER => self.src.text(node).into_owned(),
            _ => "<expr>".to_owned(),
        };
        truncate_chars(&collapse_ws(&text), MAX_CALLEE_CHARS)
    }

    /// The hint for the object of a member access.
    pub fn object_hint(&self, env: &TypeEnv, object: Node<'_>) -> Hint {
        let object = unwrap_expr(object);
        match object.kind() {
            kind::THIS => Hint::plain(ReceiverHint::This),
            kind::SUPER => Hint::plain(ReceiverHint::Super),
            kind::IDENTIFIER => self.root_hint(env, &self.src.text(object)),
            kind::MEMBER_EXPRESSION => {
                let (base, segments) = self.flatten(object);
                if base.kind() == kind::THIS && segments.len() == 1 {
                    let field_name = segments[0].clone();
                    let declared_type = env.class_field(&field_name);
                    return Hint::plain(ReceiverHint::ThisField {
                        field: field_name,
                        declared_type,
                    });
                }
                if base.kind() == kind::MEMBER_EXPRESSION {
                    // A member access missing its object or property (an error region).
                    return Hint::plain(ReceiverHint::Unknown);
                }
                let root = self.object_hint(env, base);
                Hint {
                    receiver: ReceiverHint::Chain {
                        root: Box::new(root.receiver),
                        segments,
                    },
                    binding: root.binding,
                    static_candidate: false,
                }
            }
            kind::CALL_EXPRESSION => {
                let callee = object
                    .child_by_field_name(field::FUNCTION)
                    .map(|f| self.callee_text(f))
                    .unwrap_or_default();
                Hint::plain(ReceiverHint::CallResult { callee })
            }
            kind::SUBSCRIPT_EXPRESSION => Hint::plain(ReceiverHint::Computed),
            _ => Hint::plain(ReceiverHint::Unknown),
        }
    }

    /// Splits `a.b.c` into the innermost non-member object `a` and the property names `[b, c]`.
    fn flatten<'t>(&self, mut node: Node<'t>) -> (Node<'t>, Vec<String>) {
        let mut segments: Vec<String> = Vec::new();
        loop {
            node = unwrap_expr(node);
            if node.kind() != kind::MEMBER_EXPRESSION {
                break;
            }
            let (Some(object), Some(property)) = (
                node.child_by_field_name(field::OBJECT),
                node.child_by_field_name(field::PROPERTY),
            ) else {
                break;
            };
            segments.push(self.src.text(property).into_owned());
            node = object;
        }
        segments.reverse();
        (node, segments)
    }

    /// Classifies the callee of a call, the constructor of a `new`, or a decorator expression.
    /// Returns `None` for callees that name nothing (an immediately invoked function).
    pub fn classify(&self, env: &TypeEnv, callee: Node<'_>, optional_call: bool) -> Option<Callee> {
        let callee = unwrap_expr(callee);
        let mut attrs: BTreeMap<String, AttrValue> = BTreeMap::new();
        if optional_call {
            attrs.insert(OPTIONAL.to_owned(), AttrValue::Bool(true));
        }
        let name_range = self.src.range(callee);
        match callee.kind() {
            kind::IDENTIFIER => {
                let name = self.src.text(callee).into_owned();
                let import_binding = self.binding_for(env, &name);
                Some(Callee {
                    name,
                    name_range,
                    receiver: ReceiverHint::None,
                    import_binding,
                    attrs,
                })
            }
            kind::SUPER => {
                attrs.insert("super_ctor".to_owned(), AttrValue::Bool(true));
                Some(Callee {
                    name: "super".to_owned(),
                    name_range,
                    receiver: ReceiverHint::Super,
                    import_binding: None,
                    attrs,
                })
            }
            kind::IMPORT => Some(Callee {
                name: "import".to_owned(),
                name_range,
                receiver: ReceiverHint::None,
                import_binding: None,
                attrs,
            }),
            kind::MEMBER_EXPRESSION => {
                let property = callee.child_by_field_name(field::PROPERTY)?;
                let object = callee.child_by_field_name(field::OBJECT)?;
                if has_optional_chain(callee) {
                    attrs.insert(OPTIONAL.to_owned(), AttrValue::Bool(true));
                }
                let hint = self.object_hint(env, object);
                if hint.static_candidate {
                    attrs.insert(STATIC_CANDIDATE.to_owned(), AttrValue::Bool(true));
                }
                declared_type_attrs(&hint.receiver, &mut attrs);
                Some(Callee {
                    name: self.src.text(property).into_owned(),
                    name_range: self.src.range(property),
                    receiver: hint.receiver,
                    import_binding: hint.binding,
                    attrs,
                })
            }
            kind::SUBSCRIPT_EXPRESSION => Some(Callee {
                name: "<computed>".to_owned(),
                name_range,
                receiver: ReceiverHint::Computed,
                import_binding: None,
                attrs,
            }),
            kind::FUNCTION_EXPRESSION | kind::ARROW_FUNCTION | kind::GENERATOR_FUNCTION => None,
            _ => Some(Callee {
                name: "<unknown>".to_owned(),
                name_range,
                receiver: ReceiverHint::Unknown,
                import_binding: None,
                attrs,
            }),
        }
    }

    /// Parts of a dotted name node (`A.B`), or `None` when it is not a plain path.
    pub fn parts_of(&self, node: Node<'_>) -> Option<Vec<String>> {
        member_chain(node, self.src)
    }
}

/// `declared_head` and `declared_args` for a receiver whose declared type is known.
pub fn declared_type_attrs(receiver: &ReceiverHint, attrs: &mut BTreeMap<String, AttrValue>) {
    let declared = match receiver {
        ReceiverHint::ThisField { declared_type, .. }
        | ReceiverHint::Identifier { declared_type, .. } => declared_type.as_deref(),
        _ => None,
    };
    let Some(declared) = declared else { return };
    attrs.insert(
        "declared_head".to_owned(),
        AttrValue::Str(type_head(declared).to_owned()),
    );
    let args = type_args(declared);
    if !args.is_empty() {
        attrs.insert(
            "declared_args".to_owned(),
            AttrValue::List(args.into_iter().map(AttrValue::Str).collect()),
        );
    }
}
