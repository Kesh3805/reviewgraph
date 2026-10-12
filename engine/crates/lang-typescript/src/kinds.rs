//! tree-sitter-typescript 0.23 node kinds and field names used by the visitors and the framework
//! adapters. `tests/node_kinds.rs` checks every constant against both grammars, so a grammar
//! upgrade that renames a kind fails the build instead of silently producing no symbols.

pub mod kind {
    pub const PROGRAM: &str = "program";
    pub const COMMENT: &str = "comment";
    pub const ERROR: &str = "ERROR";

    // Imports and exports.
    pub const IMPORT_STATEMENT: &str = "import_statement";
    pub const IMPORT_CLAUSE: &str = "import_clause";
    pub const NAMED_IMPORTS: &str = "named_imports";
    pub const IMPORT_SPECIFIER: &str = "import_specifier";
    pub const NAMESPACE_IMPORT: &str = "namespace_import";
    pub const IMPORT_REQUIRE_CLAUSE: &str = "import_require_clause";
    pub const EXPORT_STATEMENT: &str = "export_statement";
    pub const EXPORT_CLAUSE: &str = "export_clause";
    pub const EXPORT_SPECIFIER: &str = "export_specifier";
    pub const NAMESPACE_EXPORT: &str = "namespace_export";

    // Declarations.
    pub const CLASS_DECLARATION: &str = "class_declaration";
    pub const ABSTRACT_CLASS_DECLARATION: &str = "abstract_class_declaration";
    pub const CLASS: &str = "class";
    pub const CLASS_BODY: &str = "class_body";
    pub const CLASS_HERITAGE: &str = "class_heritage";
    pub const EXTENDS_CLAUSE: &str = "extends_clause";
    pub const IMPLEMENTS_CLAUSE: &str = "implements_clause";
    pub const INTERFACE_DECLARATION: &str = "interface_declaration";
    pub const INTERFACE_BODY: &str = "interface_body";
    pub const EXTENDS_TYPE_CLAUSE: &str = "extends_type_clause";
    pub const PROPERTY_SIGNATURE: &str = "property_signature";
    pub const METHOD_SIGNATURE: &str = "method_signature";
    pub const ABSTRACT_METHOD_SIGNATURE: &str = "abstract_method_signature";
    pub const TYPE_ALIAS_DECLARATION: &str = "type_alias_declaration";
    pub const ENUM_DECLARATION: &str = "enum_declaration";
    pub const ENUM_BODY: &str = "enum_body";
    pub const ENUM_ASSIGNMENT: &str = "enum_assignment";
    pub const FUNCTION_DECLARATION: &str = "function_declaration";
    pub const GENERATOR_FUNCTION_DECLARATION: &str = "generator_function_declaration";
    pub const FUNCTION_SIGNATURE: &str = "function_signature";
    pub const FUNCTION_EXPRESSION: &str = "function_expression";
    pub const GENERATOR_FUNCTION: &str = "generator_function";
    pub const ARROW_FUNCTION: &str = "arrow_function";
    pub const METHOD_DEFINITION: &str = "method_definition";
    pub const PUBLIC_FIELD_DEFINITION: &str = "public_field_definition";
    pub const LEXICAL_DECLARATION: &str = "lexical_declaration";
    pub const VARIABLE_DECLARATION: &str = "variable_declaration";
    pub const VARIABLE_DECLARATOR: &str = "variable_declarator";
    pub const INTERNAL_MODULE: &str = "internal_module";
    pub const MODULE: &str = "module";
    pub const AMBIENT_DECLARATION: &str = "ambient_declaration";

    // Parameters and patterns.
    pub const FORMAL_PARAMETERS: &str = "formal_parameters";
    pub const REQUIRED_PARAMETER: &str = "required_parameter";
    pub const OPTIONAL_PARAMETER: &str = "optional_parameter";
    pub const REST_PATTERN: &str = "rest_pattern";
    pub const OBJECT_PATTERN: &str = "object_pattern";
    pub const ARRAY_PATTERN: &str = "array_pattern";
    pub const PAIR_PATTERN: &str = "pair_pattern";
    pub const SHORTHAND_PROPERTY_IDENTIFIER_PATTERN: &str = "shorthand_property_identifier_pattern";
    pub const OBJECT_ASSIGNMENT_PATTERN: &str = "object_assignment_pattern";
    pub const ASSIGNMENT_PATTERN: &str = "assignment_pattern";
    pub const DECORATOR: &str = "decorator";
    pub const ACCESSIBILITY_MODIFIER: &str = "accessibility_modifier";
    pub const OVERRIDE_MODIFIER: &str = "override_modifier";

    // Expressions.
    pub const CALL_EXPRESSION: &str = "call_expression";
    pub const NEW_EXPRESSION: &str = "new_expression";
    pub const MEMBER_EXPRESSION: &str = "member_expression";
    pub const SUBSCRIPT_EXPRESSION: &str = "subscript_expression";
    pub const AWAIT_EXPRESSION: &str = "await_expression";
    pub const ASSIGNMENT_EXPRESSION: &str = "assignment_expression";
    pub const AUGMENTED_ASSIGNMENT_EXPRESSION: &str = "augmented_assignment_expression";
    pub const BINARY_EXPRESSION: &str = "binary_expression";
    pub const TERNARY_EXPRESSION: &str = "ternary_expression";
    pub const AS_EXPRESSION: &str = "as_expression";
    pub const SATISFIES_EXPRESSION: &str = "satisfies_expression";
    pub const NON_NULL_EXPRESSION: &str = "non_null_expression";
    pub const PARENTHESIZED_EXPRESSION: &str = "parenthesized_expression";
    pub const ARGUMENTS: &str = "arguments";
    pub const SPREAD_ELEMENT: &str = "spread_element";
    pub const ARRAY: &str = "array";
    pub const OBJECT: &str = "object";
    pub const PAIR: &str = "pair";
    pub const COMPUTED_PROPERTY_NAME: &str = "computed_property_name";
    pub const TEMPLATE_STRING: &str = "template_string";
    pub const TEMPLATE_SUBSTITUTION: &str = "template_substitution";
    pub const STRING: &str = "string";
    pub const STRING_FRAGMENT: &str = "string_fragment";
    pub const NUMBER: &str = "number";
    pub const TRUE: &str = "true";
    pub const FALSE: &str = "false";
    pub const NULL: &str = "null";
    pub const UNDEFINED: &str = "undefined";
    pub const THIS: &str = "this";
    pub const SUPER: &str = "super";
    pub const IMPORT: &str = "import";
    pub const JSX_ELEMENT: &str = "jsx_element";
    pub const JSX_SELF_CLOSING_ELEMENT: &str = "jsx_self_closing_element";
    pub const JSX_OPENING_ELEMENT: &str = "jsx_opening_element";

    // Identifiers and types.
    pub const IDENTIFIER: &str = "identifier";
    pub const PROPERTY_IDENTIFIER: &str = "property_identifier";
    pub const PRIVATE_PROPERTY_IDENTIFIER: &str = "private_property_identifier";
    pub const SHORTHAND_PROPERTY_IDENTIFIER: &str = "shorthand_property_identifier";
    pub const TYPE_IDENTIFIER: &str = "type_identifier";
    pub const NESTED_IDENTIFIER: &str = "nested_identifier";
    pub const NESTED_TYPE_IDENTIFIER: &str = "nested_type_identifier";
    pub const TYPE_ANNOTATION: &str = "type_annotation";
    pub const TYPE_ARGUMENTS: &str = "type_arguments";
    pub const TYPE_PARAMETERS: &str = "type_parameters";
    pub const TYPE_PARAMETER: &str = "type_parameter";
    pub const GENERIC_TYPE: &str = "generic_type";
    pub const PREDEFINED_TYPE: &str = "predefined_type";

    // Statements.
    pub const STATEMENT_BLOCK: &str = "statement_block";
    pub const EXPRESSION_STATEMENT: &str = "expression_statement";
    pub const RETURN_STATEMENT: &str = "return_statement";
    pub const THROW_STATEMENT: &str = "throw_statement";
    pub const IF_STATEMENT: &str = "if_statement";
    pub const FOR_STATEMENT: &str = "for_statement";
    pub const FOR_IN_STATEMENT: &str = "for_in_statement";
    pub const WHILE_STATEMENT: &str = "while_statement";
    pub const DO_STATEMENT: &str = "do_statement";
    pub const SWITCH_STATEMENT: &str = "switch_statement";
    pub const TRY_STATEMENT: &str = "try_statement";
    pub const CATCH_CLAUSE: &str = "catch_clause";
    pub const FINALLY_CLAUSE: &str = "finally_clause";
    pub const ELSE_CLAUSE: &str = "else_clause";
    pub const SWITCH_CASE: &str = "switch_case";
    pub const SWITCH_DEFAULT: &str = "switch_default";
    pub const UNARY_EXPRESSION: &str = "unary_expression";
}

pub mod field {
    pub const NAME: &str = "name";
    pub const BODY: &str = "body";
    pub const PARAMETERS: &str = "parameters";
    pub const PARAMETER: &str = "parameter";
    pub const RETURN_TYPE: &str = "return_type";
    pub const VALUE: &str = "value";
    pub const FUNCTION: &str = "function";
    pub const OBJECT: &str = "object";
    pub const PROPERTY: &str = "property";
    pub const ARGUMENTS: &str = "arguments";
    pub const SOURCE: &str = "source";
    pub const DECLARATION: &str = "declaration";
    pub const DECORATOR: &str = "decorator";
    pub const TYPE_PARAMETERS: &str = "type_parameters";
    pub const TYPE_ARGUMENTS: &str = "type_arguments";
    pub const CONSTRUCTOR: &str = "constructor";
    pub const LEFT: &str = "left";
    pub const RIGHT: &str = "right";
    pub const CONDITION: &str = "condition";
    pub const PATTERN: &str = "pattern";
    pub const TYPE: &str = "type";
    pub const ALIAS: &str = "alias";
    pub const KEY: &str = "key";
    pub const OPERATOR: &str = "operator";
    pub const KIND: &str = "kind";
    pub const CONSEQUENCE: &str = "consequence";
    pub const ALTERNATIVE: &str = "alternative";
    pub const HANDLER: &str = "handler";
    pub const FINALIZER: &str = "finalizer";
    pub const INDEX: &str = "index";
}

/// Every kind constant, for the grammar-consistency test.
pub const ALL_KINDS: &[&str] = &[
    kind::PROGRAM,
    kind::COMMENT,
    kind::IMPORT_STATEMENT,
    kind::IMPORT_CLAUSE,
    kind::NAMED_IMPORTS,
    kind::IMPORT_SPECIFIER,
    kind::NAMESPACE_IMPORT,
    kind::IMPORT_REQUIRE_CLAUSE,
    kind::EXPORT_STATEMENT,
    kind::EXPORT_CLAUSE,
    kind::EXPORT_SPECIFIER,
    kind::NAMESPACE_EXPORT,
    kind::CLASS_DECLARATION,
    kind::ABSTRACT_CLASS_DECLARATION,
    kind::CLASS,
    kind::CLASS_BODY,
    kind::CLASS_HERITAGE,
    kind::EXTENDS_CLAUSE,
    kind::IMPLEMENTS_CLAUSE,
    kind::INTERFACE_DECLARATION,
    kind::INTERFACE_BODY,
    kind::EXTENDS_TYPE_CLAUSE,
    kind::PROPERTY_SIGNATURE,
    kind::METHOD_SIGNATURE,
    kind::ABSTRACT_METHOD_SIGNATURE,
    kind::TYPE_ALIAS_DECLARATION,
    kind::ENUM_DECLARATION,
    kind::ENUM_BODY,
    kind::ENUM_ASSIGNMENT,
    kind::FUNCTION_DECLARATION,
    kind::GENERATOR_FUNCTION_DECLARATION,
    kind::FUNCTION_SIGNATURE,
    kind::FUNCTION_EXPRESSION,
    kind::GENERATOR_FUNCTION,
    kind::ARROW_FUNCTION,
    kind::METHOD_DEFINITION,
    kind::PUBLIC_FIELD_DEFINITION,
    kind::LEXICAL_DECLARATION,
    kind::VARIABLE_DECLARATION,
    kind::VARIABLE_DECLARATOR,
    kind::INTERNAL_MODULE,
    kind::MODULE,
    kind::AMBIENT_DECLARATION,
    kind::FORMAL_PARAMETERS,
    kind::REQUIRED_PARAMETER,
    kind::OPTIONAL_PARAMETER,
    kind::REST_PATTERN,
    kind::OBJECT_PATTERN,
    kind::ARRAY_PATTERN,
    kind::PAIR_PATTERN,
    kind::SHORTHAND_PROPERTY_IDENTIFIER_PATTERN,
    kind::OBJECT_ASSIGNMENT_PATTERN,
    kind::ASSIGNMENT_PATTERN,
    kind::DECORATOR,
    kind::ACCESSIBILITY_MODIFIER,
    kind::OVERRIDE_MODIFIER,
    kind::CALL_EXPRESSION,
    kind::NEW_EXPRESSION,
    kind::MEMBER_EXPRESSION,
    kind::SUBSCRIPT_EXPRESSION,
    kind::AWAIT_EXPRESSION,
    kind::ASSIGNMENT_EXPRESSION,
    kind::AUGMENTED_ASSIGNMENT_EXPRESSION,
    kind::BINARY_EXPRESSION,
    kind::TERNARY_EXPRESSION,
    kind::AS_EXPRESSION,
    kind::SATISFIES_EXPRESSION,
    kind::NON_NULL_EXPRESSION,
    kind::PARENTHESIZED_EXPRESSION,
    kind::ARGUMENTS,
    kind::SPREAD_ELEMENT,
    kind::ARRAY,
    kind::OBJECT,
    kind::PAIR,
    kind::COMPUTED_PROPERTY_NAME,
    kind::TEMPLATE_STRING,
    kind::TEMPLATE_SUBSTITUTION,
    kind::STRING,
    kind::STRING_FRAGMENT,
    kind::NUMBER,
    kind::TRUE,
    kind::FALSE,
    kind::NULL,
    kind::UNDEFINED,
    kind::THIS,
    kind::SUPER,
    kind::IMPORT,
    kind::JSX_ELEMENT,
    kind::JSX_SELF_CLOSING_ELEMENT,
    kind::JSX_OPENING_ELEMENT,
    kind::IDENTIFIER,
    kind::PROPERTY_IDENTIFIER,
    kind::PRIVATE_PROPERTY_IDENTIFIER,
    kind::SHORTHAND_PROPERTY_IDENTIFIER,
    kind::TYPE_IDENTIFIER,
    kind::NESTED_IDENTIFIER,
    kind::NESTED_TYPE_IDENTIFIER,
    kind::TYPE_ANNOTATION,
    kind::TYPE_ARGUMENTS,
    kind::TYPE_PARAMETERS,
    kind::TYPE_PARAMETER,
    kind::GENERIC_TYPE,
    kind::PREDEFINED_TYPE,
    kind::STATEMENT_BLOCK,
    kind::EXPRESSION_STATEMENT,
    kind::RETURN_STATEMENT,
    kind::THROW_STATEMENT,
    kind::IF_STATEMENT,
    kind::FOR_STATEMENT,
    kind::FOR_IN_STATEMENT,
    kind::WHILE_STATEMENT,
    kind::DO_STATEMENT,
    kind::SWITCH_STATEMENT,
    kind::TRY_STATEMENT,
    kind::CATCH_CLAUSE,
    kind::FINALLY_CLAUSE,
    kind::ELSE_CLAUSE,
    kind::SWITCH_CASE,
    kind::SWITCH_DEFAULT,
    kind::UNARY_EXPRESSION,
];

/// Every field-name constant, for the grammar-consistency test.
pub const ALL_FIELDS: &[&str] = &[
    field::NAME,
    field::BODY,
    field::PARAMETERS,
    field::PARAMETER,
    field::RETURN_TYPE,
    field::VALUE,
    field::FUNCTION,
    field::OBJECT,
    field::PROPERTY,
    field::ARGUMENTS,
    field::SOURCE,
    field::DECLARATION,
    field::DECORATOR,
    field::TYPE_PARAMETERS,
    field::TYPE_ARGUMENTS,
    field::CONSTRUCTOR,
    field::LEFT,
    field::RIGHT,
    field::CONDITION,
    field::PATTERN,
    field::TYPE,
    field::ALIAS,
    field::KEY,
    field::OPERATOR,
    field::KIND,
    field::CONSEQUENCE,
    field::ALTERNATIVE,
    field::HANDLER,
    field::FINALIZER,
    field::INDEX,
];
