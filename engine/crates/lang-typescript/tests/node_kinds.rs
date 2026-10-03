#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lang_typescript::kinds::{ALL_FIELDS, ALL_KINDS};
use lang_typescript::parser_pool::Grammar;

/// JSX exists only in the TSX grammar; every other constant must exist in both.
const TSX_ONLY: &[&str] = &[
    "jsx_element",
    "jsx_self_closing_element",
    "jsx_opening_element",
];

#[test]
fn node_kinds_exist_in_grammar() {
    for grammar in [Grammar::Typescript, Grammar::Tsx] {
        let language = grammar.language();
        for kind in ALL_KINDS {
            if grammar == Grammar::Typescript && TSX_ONLY.contains(kind) {
                continue;
            }
            let named_id = language.id_for_node_kind(kind, true);
            let anon_id = language.id_for_node_kind(kind, false);
            assert!(
                named_id != 0 || anon_id != 0,
                "kind `{kind}` does not exist in the {grammar:?} grammar"
            );
        }
    }
}

#[test]
fn field_names_exist_in_grammar() {
    for grammar in [Grammar::Typescript, Grammar::Tsx] {
        let language = grammar.language();
        for field in ALL_FIELDS {
            assert!(
                language.field_id_for_name(field).is_some(),
                "field `{field}` does not exist in the {grammar:?} grammar"
            );
        }
    }
}
