//! References from symbols to names, with receiver hints for the linker.

use std::collections::BTreeMap;

use review_core::location::SourceRange;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::symbol::{AttrValue, LocalId};

/// `(import index, binding index)` into `ParsedUnit::imports`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct BindingRef {
    pub import: u32,
    pub binding: u32,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum RefKind {
    Call,
    New,
    TypeRef,
    Extends,
    Implements,
    Decorator,
    DiInjection,
    FrameworkRef,
    ValueRead,
    JsxElement,
}

impl RefKind {
    pub const ALL: [RefKind; 10] = [
        Self::Call,
        Self::New,
        Self::TypeRef,
        Self::Extends,
        Self::Implements,
        Self::Decorator,
        Self::DiInjection,
        Self::FrameworkRef,
        Self::ValueRead,
        Self::JsxElement,
    ];

    /// Stable string form, used in graph edge properties.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::New => "new",
            Self::TypeRef => "type_ref",
            Self::Extends => "extends",
            Self::Implements => "implements",
            Self::Decorator => "decorator",
            Self::DiInjection => "di_injection",
            Self::FrameworkRef => "framework_ref",
            Self::ValueRead => "value_read",
            Self::JsxElement => "jsx_element",
        }
    }
}

/// What is known about the object a member is accessed on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ReceiverHint {
    None,
    This,
    Super,
    ThisField {
        field: String,
        declared_type: Option<String>,
    },
    Identifier {
        name: String,
        declared_type: Option<String>,
    },
    ImportedNamespace {
        binding: BindingRef,
    },
    Chain {
        root: Box<ReceiverHint>,
        segments: Vec<String>,
    },
    CallResult {
        callee: String,
    },
    Computed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IrReference {
    pub from: LocalId,
    pub kind: RefKind,
    /// Terminal identifier.
    pub name: String,
    pub receiver: ReceiverHint,
    /// Set when `name` or the receiver root is an imported binding.
    pub import_binding: Option<BindingRef>,
    pub range: SourceRange,
    pub arg_count: u16,
    pub in_test_block: bool,
    pub attrs: BTreeMap<String, AttrValue>,
}
