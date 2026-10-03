//! Symbols and the bounded expression tree attached to them.

use std::collections::BTreeMap;

use review_core::location::SourceRange;
use review_core::symbol::{Hash128, SymbolKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Index of a symbol inside its [`crate::ParsedUnit`]. `symbols[local_id.0]` is the symbol.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct LocalId(pub u32);

/// Qualified name segments, outermost first (`["AuthService", "authorize"]`). The module symbol
/// uses `["__module__"]`.
pub type QualifiedName = Vec<String>;

/// Modifier bit set. Bit positions are part of the cached IR format (`IR_SCHEMA_VERSION`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Modifiers(pub u16);

impl Modifiers {
    pub const EXPORTED: Self = Self(1 << 0);
    pub const DEFAULT_EXPORT: Self = Self(1 << 1);
    pub const ASYNC: Self = Self(1 << 2);
    pub const STATIC: Self = Self(1 << 3);
    pub const ABSTRACT: Self = Self(1 << 4);
    pub const READONLY: Self = Self(1 << 5);
    pub const DECLARE: Self = Self(1 << 6);
    pub const GENERATOR: Self = Self(1 << 7);
    pub const OPTIONAL: Self = Self(1 << 8);
    pub const OVERRIDE: Self = Self(1 << 9);
    pub const CONST_ENUM: Self = Self(1 << 10);
    pub const AMBIENT: Self = Self(1 << 11);

    pub const NAMES: [(&'static str, Self); 12] = [
        ("exported", Self::EXPORTED),
        ("default_export", Self::DEFAULT_EXPORT),
        ("async", Self::ASYNC),
        ("static", Self::STATIC),
        ("abstract", Self::ABSTRACT),
        ("readonly", Self::READONLY),
        ("declare", Self::DECLARE),
        ("generator", Self::GENERATOR),
        ("optional", Self::OPTIONAL),
        ("override", Self::OVERRIDE),
        ("const_enum", Self::CONST_ENUM),
        ("ambient", Self::AMBIENT),
    ];

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Names of the set bits, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(_, m)| self.contains(*m))
            .map(|(n, _)| *n)
            .collect()
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum Visibility {
    #[default]
    Public,
    Protected,
    Private,
    /// ECMAScript `#private` member.
    EcmaPrivate,
}

/// Limits of [`IrExpr`]: decorator arguments and constants are kept as bounded literal trees so
/// adapters never need AST nodes.
pub const MAX_EXPR_DEPTH: usize = 6;
pub const MAX_EXPR_ELEMENTS: usize = 64;
pub const MAX_EXPR_STRING_BYTES: usize = 1024;
pub const MAX_EXPR_OTHER_CHARS: usize = 120;

/// A bounded literal expression tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IrExpr {
    Str(String),
    Num(String),
    Bool(bool),
    Null,
    Ident(String),
    Member(Vec<String>),
    Array(Vec<IrExpr>),
    Object(Vec<(String, IrExpr)>),
    Call {
        callee: Vec<String>,
        args: Vec<IrExpr>,
    },
    /// `() => X`
    Arrow {
        returns: Box<IrExpr>,
    },
    Template {
        raw: String,
        has_subst: bool,
    },
    /// Anything else, or a truncated subtree (`"…"`). At most 120 characters.
    Other(String),
}

impl IrExpr {
    /// The marker that replaces anything beyond the limits.
    pub fn truncated() -> Self {
        Self::Other("…".to_owned())
    }

    /// Longest path of nested expressions (a leaf has depth 1).
    pub fn depth(&self) -> usize {
        match self {
            Self::Array(items) => 1 + items.iter().map(Self::depth).max().unwrap_or(0),
            Self::Object(pairs) => 1 + pairs.iter().map(|(_, v)| v.depth()).max().unwrap_or(0),
            Self::Call { args, .. } => 1 + args.iter().map(Self::depth).max().unwrap_or(0),
            Self::Arrow { returns } => 1 + returns.depth(),
            _ => 1,
        }
    }

    /// Rewrites the tree so it respects depth, element-count and string-length limits.
    /// Cut points become `Other("…")`.
    pub fn bounded(self) -> Self {
        self.limit(1)
    }

    fn limit(self, depth: usize) -> Self {
        if depth > MAX_EXPR_DEPTH {
            return Self::truncated();
        }
        let cut = |s: String, max: usize| -> String {
            if s.len() <= max {
                return s;
            }
            let mut end = max;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            s[..end].to_owned()
        };
        match self {
            Self::Str(s) => Self::Str(cut(s, MAX_EXPR_STRING_BYTES)),
            Self::Num(s) => Self::Num(cut(s, MAX_EXPR_OTHER_CHARS)),
            Self::Ident(s) => Self::Ident(cut(s, MAX_EXPR_OTHER_CHARS)),
            Self::Template { raw, has_subst } => Self::Template {
                raw: cut(raw, MAX_EXPR_STRING_BYTES),
                has_subst,
            },
            Self::Other(s) => Self::Other(s.chars().take(MAX_EXPR_OTHER_CHARS).collect()),
            Self::Member(parts) => {
                Self::Member(parts.into_iter().take(MAX_EXPR_ELEMENTS).collect())
            }
            Self::Array(items) => {
                let overflow = items.len() > MAX_EXPR_ELEMENTS;
                let mut out: Vec<Self> = items
                    .into_iter()
                    .take(MAX_EXPR_ELEMENTS)
                    .map(|e| e.limit(depth + 1))
                    .collect();
                if overflow {
                    out.pop();
                    out.push(Self::truncated());
                }
                Self::Array(out)
            }
            Self::Object(pairs) => {
                let overflow = pairs.len() > MAX_EXPR_ELEMENTS;
                let mut out: Vec<(String, Self)> = pairs
                    .into_iter()
                    .take(MAX_EXPR_ELEMENTS)
                    .map(|(k, v)| (cut(k, MAX_EXPR_OTHER_CHARS), v.limit(depth + 1)))
                    .collect();
                if overflow {
                    out.pop();
                    out.push(("…".to_owned(), Self::truncated()));
                }
                Self::Object(out)
            }
            Self::Call { callee, args } => {
                let overflow = args.len() > MAX_EXPR_ELEMENTS;
                let mut out: Vec<Self> = args
                    .into_iter()
                    .take(MAX_EXPR_ELEMENTS)
                    .map(|e| e.limit(depth + 1))
                    .collect();
                if overflow {
                    out.pop();
                    out.push(Self::truncated());
                }
                Self::Call {
                    callee: callee.into_iter().take(MAX_EXPR_ELEMENTS).collect(),
                    args: out,
                }
            }
            Self::Arrow { returns } => Self::Arrow {
                returns: Box::new(returns.limit(depth + 1)),
            },
            leaf @ (Self::Bool(_) | Self::Null) => leaf,
        }
    }
}

/// A decorator application (`@Get(":id")`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IrDecorator {
    /// Callee text such as `Get` or `Nest.Get`.
    pub name: String,
    pub args: Vec<IrExpr>,
    pub range: SourceRange,
}

/// TypeScript constructor parameter property (`constructor(private readonly x: X)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParamProperty {
    pub visibility: Visibility,
    pub readonly: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IrParam {
    pub name: String,
    pub type_text: Option<String>,
    pub optional: bool,
    pub rest: bool,
    pub decorators: Vec<IrDecorator>,
    pub property: Option<ParamProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Heritage {
    pub extends: Vec<String>,
    pub implements: Vec<String>,
}

/// Literal value of a constant or enum member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ConstValue {
    Str(String),
    Num(String),
    Bool(bool),
}

/// An `f64` stored as its IEEE bits so attribute maps stay `Eq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FloatBits(pub u64);

impl FloatBits {
    pub fn new(value: f64) -> Self {
        Self(value.to_bits())
    }

    pub fn get(self) -> f64 {
        f64::from_bits(self.0)
    }
}

/// Free-form, deterministic attribute values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AttrValue {
    Str(String),
    Int(i64),
    Float(FloatBits),
    Bool(bool),
    List(Vec<AttrValue>),
    Map(BTreeMap<String, AttrValue>),
    Expr(IrExpr),
}

/// Normalized body shingles (TSA-007): a sorted, de-duplicated sample of token n-gram hashes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ShingleSet(pub Vec<u32>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IrSymbol {
    pub local_id: LocalId,
    pub kind: SymbolKind,
    pub name: String,
    pub qualified_name: QualifiedName,
    /// Overload/duplicate ordinal (SID-003); 0 means none.
    pub ordinal: u16,
    pub parent: Option<LocalId>,
    pub range: SourceRange,
    pub name_range: Option<SourceRange>,
    pub body_range: Option<SourceRange>,
    /// Display text, at most 512 characters, single-spaced.
    pub signature: Option<String>,
    pub signature_hash: Hash128,
    pub body_hash: Hash128,
    pub attr_hash: Hash128,
    pub body_shingles: ShingleSet,
    pub body_token_count: u32,
    pub modifiers: Modifiers,
    pub visibility: Visibility,
    pub decorators: Vec<IrDecorator>,
    pub type_params: Vec<String>,
    pub params: Vec<IrParam>,
    pub return_type: Option<String>,
    pub heritage: Heritage,
    pub declared_type: Option<String>,
    pub const_value: Option<ConstValue>,
    pub overload_signatures: Vec<String>,
    pub doc_hash: Option<Hash128>,
    pub has_errors: bool,
    pub attrs: BTreeMap<String, AttrValue>,
}

impl IrSymbol {
    /// A symbol with every optional part empty, for analyzers to fill in.
    pub fn new(
        local_id: LocalId,
        kind: SymbolKind,
        name: impl Into<String>,
        qualified_name: QualifiedName,
        range: SourceRange,
    ) -> Self {
        Self {
            local_id,
            kind,
            name: name.into(),
            qualified_name,
            ordinal: 0,
            parent: None,
            range,
            name_range: None,
            body_range: None,
            signature: None,
            signature_hash: Hash128::ZERO,
            body_hash: Hash128::ZERO,
            attr_hash: Hash128::ZERO,
            body_shingles: ShingleSet::default(),
            body_token_count: 0,
            modifiers: Modifiers::empty(),
            visibility: Visibility::Public,
            decorators: Vec::new(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            heritage: Heritage::default(),
            declared_type: None,
            const_value: None,
            overload_signatures: Vec::new(),
            doc_hash: None,
            has_errors: false,
            attrs: BTreeMap::new(),
        }
    }
}
