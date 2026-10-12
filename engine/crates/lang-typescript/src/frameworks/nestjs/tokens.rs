//! Injection-token classification shared by the DI facts (NEST-003), the TypeORM access
//! heuristics (NEST-005) and the BullMQ producers (NEST-006).

use std::collections::BTreeMap;

use analysis_ir::{IrDecorator, IrExpr, LocalId};
use review_core::symbol::SymbolKind;

use crate::frameworks::util::{class_ident, name_of};
use crate::frameworks::FrameworkCtx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Class,
    Str,
    Symbol,
    ForwardRef,
    TypeormRepository,
    TypeormEntityManager,
    TypeormDataSource,
    BullmqQueue,
    Model,
    Unknown,
}

impl TokenKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Str => "string",
            Self::Symbol => "symbol",
            Self::ForwardRef => "forward_ref",
            Self::TypeormRepository => "typeorm_repository",
            Self::TypeormEntityManager => "typeorm_entity_manager",
            Self::TypeormDataSource => "typeorm_data_source",
            Self::BullmqQueue => "bullmq_queue",
            Self::Model => "model",
            Self::Unknown => "unknown",
        }
    }
}

/// What the `@Inject*` decorators of one parameter or property say.
#[derive(Debug, Clone, Default)]
pub struct InjectedToken {
    pub kind: Option<TokenKind>,
    pub token: Option<IrExpr>,
    /// Entity class of a repository or model token.
    pub entity: Option<String>,
    /// Queue name expression of an `@InjectQueue`.
    pub queue: Option<IrExpr>,
    pub optional: bool,
    pub scope_flags: Vec<String>,
    /// An `@Inject*` decorator that is not bound to a `@nestjs/*` package.
    pub unbound: bool,
    /// Any `@Inject*` decorator was present.
    pub explicit: bool,
}

/// The exported name behind a decorator or callee name and whether it is bound to `@nestjs/*`;
/// `None` when it is bound to something else.
pub fn nest_name(ctx: &FrameworkCtx<'_>, name: &str) -> Option<(String, bool)> {
    let mut parts = name.split('.');
    let root = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    match (ctx.binding(root), rest.as_slice()) {
        (Some(b), []) if b.specifier.starts_with("@nestjs/") => Some((b.imported, true)),
        (Some(b), [last]) if b.specifier.starts_with("@nestjs/") && b.imported == "*" => {
            Some(((*last).to_owned(), true))
        }
        (None, []) => Some((root.to_owned(), false)),
        _ => None,
    }
}

fn is_all_caps(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Classifies the `@Inject*`, `@Optional`, `@Self`, `@SkipSelf` and `@Host` decorators.
pub fn classify(ctx: &FrameworkCtx<'_>, decorators: &[IrDecorator]) -> InjectedToken {
    let mut out = InjectedToken::default();
    for decorator in decorators {
        let Some((real, bound)) = nest_name(ctx, &decorator.name) else {
            continue;
        };
        let first = decorator.args.first();
        let kind = match real.as_str() {
            "Inject" => {
                let kind = match first {
                    Some(IrExpr::Call { callee, args })
                        if callee.last().map(String::as_str) == Some("forwardRef") =>
                    {
                        if let Some(IrExpr::Arrow { returns }) = args.first() {
                            out.entity = Some(name_of(returns));
                        }
                        TokenKind::ForwardRef
                    }
                    Some(IrExpr::Call { callee, args }) => {
                        match callee.last().map(String::as_str) {
                            Some("getRepositoryToken") => {
                                out.entity = args.first().map(name_of);
                                TokenKind::TypeormRepository
                            }
                            Some("getQueueToken") => {
                                out.queue = args.first().cloned();
                                TokenKind::BullmqQueue
                            }
                            Some("getDataSourceToken" | "getConnectionToken") => {
                                TokenKind::TypeormDataSource
                            }
                            Some("getEntityManagerToken") => TokenKind::TypeormEntityManager,
                            _ => TokenKind::Unknown,
                        }
                    }
                    Some(IrExpr::Str(_) | IrExpr::Template { .. }) => TokenKind::Str,
                    Some(IrExpr::Ident(n))
                        if ctx.const_str(&IrExpr::Ident(n.clone())).is_some() =>
                    {
                        TokenKind::Str
                    }
                    Some(IrExpr::Ident(n)) if is_all_caps(n) => TokenKind::Symbol,
                    Some(IrExpr::Member(_)) => TokenKind::Symbol,
                    Some(expr) if class_ident(expr).is_some() => TokenKind::Class,
                    _ => TokenKind::Unknown,
                };
                out.token = first.cloned();
                Some(kind)
            }
            "InjectRepository" => {
                out.entity = first.map(name_of).filter(|n| !n.is_empty());
                out.token = first.cloned();
                Some(TokenKind::TypeormRepository)
            }
            "InjectEntityManager" => Some(TokenKind::TypeormEntityManager),
            "InjectDataSource" | "InjectConnection" => Some(TokenKind::TypeormDataSource),
            "InjectQueue" => {
                out.queue = first.cloned();
                Some(TokenKind::BullmqQueue)
            }
            "InjectModel" => {
                out.entity = first.map(name_of).filter(|n| !n.is_empty());
                out.token = first.cloned();
                Some(TokenKind::Model)
            }
            "Optional" => {
                if bound || decorator.name == "Optional" {
                    out.optional = true;
                }
                None
            }
            "Self" => {
                out.scope_flags.push("self".to_owned());
                None
            }
            "SkipSelf" => {
                out.scope_flags.push("skip_self".to_owned());
                None
            }
            "Host" => {
                out.scope_flags.push("host".to_owned());
                None
            }
            _ => None,
        };
        if let Some(kind) = kind {
            out.kind = Some(kind);
            out.explicit = true;
            if !bound {
                out.unbound = true;
            }
        }
    }
    out
}

/// Injected tokens keyed by owning class and property name.
pub type InjectedMap = BTreeMap<(LocalId, String), InjectedToken>;

/// Injected tokens of constructor parameter properties and decorated properties, keyed by the
/// owning class and the property name.
pub fn injected_properties(ctx: &FrameworkCtx<'_>) -> InjectedMap {
    let mut out = BTreeMap::new();
    for symbol in ctx.symbols {
        match symbol.kind {
            SymbolKind::Constructor => {
                let Some(class) = symbol.parent else { continue };
                for param in &symbol.params {
                    if param.property.is_none() {
                        continue;
                    }
                    let token = classify(ctx, &param.decorators);
                    if token.explicit {
                        out.insert((class, param.name.clone()), token);
                    }
                }
            }
            SymbolKind::Property | SymbolKind::Field => {
                let Some(class) = symbol.parent else { continue };
                let token = classify(ctx, &symbol.decorators);
                if token.explicit {
                    out.insert((class, symbol.name.clone()), token);
                }
            }
            _ => {}
        }
    }
    out
}
