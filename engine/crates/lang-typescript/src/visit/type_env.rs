//! The local type environment of the reference pass (TSA-005): which names are declared in the
//! enclosing function scopes and with what type, which fields the enclosing class declares, and
//! which type parameters are in scope. It records only direct forms; nothing is inferred.

use std::collections::BTreeMap;

use tree_sitter::Node;

use super::expr::{collapse_ws, truncate_chars};
use crate::kinds::{field, kind};
use crate::text::Source;

/// Longest declared-type text kept.
const MAX_TYPE_CHARS: usize = 256;

const ACCESSIBILITY_MODIFIER: &str = "accessibility_modifier";
const READONLY: &str = "readonly";
const FORMAL_PARAMETERS: &str = "formal_parameters";
const CONSTRUCTOR: &str = "constructor";

/// Scoped declarations. Scopes are plain vectors, so shadowing is "innermost scope wins".
#[derive(Debug)]
pub struct TypeEnv {
    /// Name to declared type; `None` means "declared here, type unknown" (which still shadows).
    scopes: Vec<BTreeMap<String, Option<String>>>,
    /// Field name to declared type, one map per enclosing class.
    classes: Vec<BTreeMap<String, String>>,
    generics: Vec<String>,
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeEnv {
    pub fn new() -> Self {
        Self {
            scopes: vec![BTreeMap::new()],
            classes: Vec::new(),
            generics: Vec::new(),
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    pub fn declare(&mut self, name: &str, declared: Option<String>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned(), declared);
        }
    }

    pub fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains_key(name))
    }

    /// The declared type of the innermost declaration of `name`, if it has one.
    pub fn declared_type(&self, name: &str) -> Option<String> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .and_then(|declared| declared.clone())
    }

    pub fn push_class(&mut self, fields: BTreeMap<String, String>) {
        self.classes.push(fields);
    }

    pub fn pop_class(&mut self) {
        self.classes.pop();
    }

    /// The declared type of `this.<field>` in the innermost class.
    pub fn class_field(&self, name: &str) -> Option<String> {
        self.classes.last().and_then(|c| c.get(name)).cloned()
    }

    pub fn generics_mark(&self) -> usize {
        self.generics.len()
    }

    pub fn add_generic(&mut self, name: String) {
        self.generics.push(name);
    }

    pub fn truncate_generics(&mut self, mark: usize) {
        self.generics.truncate(mark);
    }

    pub fn is_generic(&self, name: &str) -> bool {
        self.generics.iter().any(|g| g == name)
    }
}

/// The text of the type a `type_annotation` holds (`: Repository<User>` gives `Repository<User>`).
pub fn annotation_text(annotation: Node<'_>, src: &Source<'_>) -> Option<String> {
    let inner = annotation.named_child(0)?;
    let text = truncate_chars(&collapse_ws(&src.text(inner)), MAX_TYPE_CHARS);
    (!text.is_empty()).then_some(text)
}

/// The declared type of a node with a `type` field (a declarator, a parameter, a field).
pub fn declared_type_of(node: Node<'_>, src: &Source<'_>) -> Option<String> {
    let annotation = node.child_by_field_name(field::TYPE)?;
    annotation_text(annotation, src)
}

/// `new T<A>(...)` gives `T<A>`.
pub fn new_target_text(value: Node<'_>, src: &Source<'_>) -> Option<String> {
    if value.kind() != kind::NEW_EXPRESSION {
        return None;
    }
    let target = value.child_by_field_name(field::CONSTRUCTOR)?;
    if !matches!(target.kind(), kind::IDENTIFIER | kind::MEMBER_EXPRESSION) {
        return None;
    }
    let mut text = src.text(target).into_owned();
    if let Some(arguments) = value.child_by_field_name(field::TYPE_ARGUMENTS) {
        text.push_str(&src.text(arguments));
    }
    Some(truncate_chars(&collapse_ws(&text), MAX_TYPE_CHARS))
}

/// Field name to declared type for one class body: typed fields, fields initialized with `new T`
/// and constructor parameter properties.
pub fn build_class_env(body: Node<'_>, src: &Source<'_>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        match member.kind() {
            kind::PUBLIC_FIELD_DEFINITION => {
                let Some(name) = member.child_by_field_name(field::NAME) else {
                    continue;
                };
                let declared = declared_type_of(member, src).or_else(|| {
                    member
                        .child_by_field_name(field::VALUE)
                        .and_then(|value| new_target_text(value, src))
                });
                if let Some(declared) = declared {
                    env.entry(src.text(name).into_owned()).or_insert(declared);
                }
            }
            kind::METHOD_DEFINITION => {
                let is_constructor = member
                    .child_by_field_name(field::NAME)
                    .is_some_and(|n| src.text(n) == CONSTRUCTOR);
                if !is_constructor {
                    continue;
                }
                if let Some(params) = member.child_by_field_name(field::PARAMETERS) {
                    add_parameter_properties(params, src, &mut env);
                }
            }
            _ => {}
        }
    }
    env
}

fn add_parameter_properties(
    params: Node<'_>,
    src: &Source<'_>,
    env: &mut BTreeMap<String, String>,
) {
    if params.kind() != FORMAL_PARAMETERS {
        return;
    }
    let mut cursor = params.walk();
    for param in params.named_children(&mut cursor) {
        if !matches!(
            param.kind(),
            kind::REQUIRED_PARAMETER | kind::OPTIONAL_PARAMETER
        ) {
            continue;
        }
        let mut inner = param.walk();
        let is_property = param
            .children(&mut inner)
            .any(|c| c.kind() == ACCESSIBILITY_MODIFIER || c.kind() == READONLY);
        if !is_property {
            continue;
        }
        let Some(pattern) = param.child_by_field_name(field::PATTERN) else {
            continue;
        };
        if let Some(declared) = declared_type_of(param, src) {
            env.entry(src.text(pattern).into_owned())
                .or_insert(declared);
        }
    }
}

/// The head of a declared type: `Repository<User>` gives `Repository`.
pub fn type_head(declared: &str) -> &str {
    declared.split('<').next().unwrap_or(declared).trim()
}

/// The top-level type arguments of a declared type: `Map<string, List<A>>` gives
/// `["string", "List<A>"]`.
pub fn type_args(declared: &str) -> Vec<String> {
    let Some(open) = declared.find('<') else {
        return Vec::new();
    };
    let Some(close) = declared.rfind('>') else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in declared[open + 1..close].chars() {
        match ch {
            '<' | '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            '>' | ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                out.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_and_args_of_declared_types() {
        assert_eq!(type_head("Repository<User>"), "Repository");
        assert_eq!(type_head("Plain"), "Plain");
        assert_eq!(
            type_args("Map<string, List<A, B>>"),
            vec!["string", "List<A, B>"]
        );
        assert!(type_args("Plain").is_empty());
    }

    #[test]
    fn inner_scopes_shadow_outer_ones() {
        let mut env = TypeEnv::new();
        env.declare("repo", Some("A".to_owned()));
        env.push_scope();
        assert_eq!(env.declared_type("repo").as_deref(), Some("A"));
        env.declare("repo", Some("B".to_owned()));
        assert_eq!(env.declared_type("repo").as_deref(), Some("B"));
        env.declare("other", None);
        assert!(env.is_local("other"));
        env.pop_scope();
        assert_eq!(env.declared_type("repo").as_deref(), Some("A"));
        assert!(!env.is_local("other"));
    }
}
