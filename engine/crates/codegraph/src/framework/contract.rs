//! The attribute contract analyzers must satisfy (CG-006).
//!
//! Framework knowledge stays in `lang-typescript`; what crosses the boundary is a
//! [`IrFrameworkFact`](analysis_ir::framework::IrFrameworkFact) — a category plus a
//! `BTreeMap<String, AttrValue>`. This module is the written contract for those attributes:
//! which ones each category requires, how they are read, and what a fact that does not satisfy
//! it turns into.
//!
//! A missing or malformed attribute is a [`FactIssue`], never an error: a framework adapter that
//! emits slightly-wrong attributes must not fail an index run (CG-006 "Failure behavior"). The
//! issue travels to IDX-003 and is persisted as a parse diagnostic.

use std::fmt;

use analysis_ir::framework::{FrameworkFactKind, IrFrameworkFact};
use analysis_ir::symbol::AttrValue;
use review_core::location::{RepoPath, SourceRange};

/// The framework-neutral categories this mapper understands.
///
/// `analysis_ir` has one variant per *adapter* construct; several of those map onto the same
/// generic category (a queue consumer class and a queue job handler are both `queue_consumer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FactCategory {
    Controller,
    Route,
    Guard,
    QueueProducer,
    QueueConsumer,
    Entity,
    DbAccess,
    EnvRead,
    TestSuite,
    TestCase,
    GlobalPrefix,
}

impl FactCategory {
    /// Every category, for the documentation test and the counter labels.
    pub const ALL: [FactCategory; 11] = [
        FactCategory::Controller,
        FactCategory::Route,
        FactCategory::Guard,
        FactCategory::QueueProducer,
        FactCategory::QueueConsumer,
        FactCategory::Entity,
        FactCategory::DbAccess,
        FactCategory::EnvRead,
        FactCategory::TestSuite,
        FactCategory::TestCase,
        FactCategory::GlobalPrefix,
    ];

    /// Lower-snake label used by the `framework_facts_total{category}` counter.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Route => "route",
            Self::Guard => "guard",
            Self::QueueProducer => "queue_producer",
            Self::QueueConsumer => "queue_consumer",
            Self::Entity => "entity",
            Self::DbAccess => "db_access",
            Self::EnvRead => "env_read",
            Self::TestSuite => "test_suite",
            Self::TestCase => "test_case",
            Self::GlobalPrefix => "global_prefix",
        }
    }

    /// The attributes a conforming analyzer must set, documented and asserted by a test.
    pub const REQUIRED: &'static [(&'static str, &'static str)] = &[
        ("controller", "symbol"),
        ("route", "symbol,method,path"),
        ("guard", "symbol"),
        ("queue_producer", "symbol,queue"),
        ("queue_consumer", "symbol,queue"),
        ("entity", "symbol,table"),
        ("db_access", "symbol,entity,op"),
        ("env_read", "symbol,name"),
        ("test_suite", "suite_path,name,range"),
        ("test_case", "suite_path,name,range"),
        ("global_prefix", "prefix"),
    ];
}

impl fmt::Display for FactCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Maps an IR fact kind onto a generic category, or `None` when the fact carries no graph
/// construct (`ModuleDeclaration`, `DiInjection`, `OrmColumn`, `TestMock`, …).
#[must_use]
pub fn category_of(kind: &FrameworkFactKind) -> Option<FactCategory> {
    match kind {
        FrameworkFactKind::Controller => Some(FactCategory::Controller),
        FrameworkFactKind::HttpRoute | FrameworkFactKind::RouteMetadata => {
            Some(FactCategory::Route)
        }
        FrameworkFactKind::Middleware | FrameworkFactKind::MiddlewareBinding => {
            Some(FactCategory::Guard)
        }
        FrameworkFactKind::QueueProducer => Some(FactCategory::QueueProducer),
        FrameworkFactKind::QueueConsumer | FrameworkFactKind::QueueJobHandler => {
            Some(FactCategory::QueueConsumer)
        }
        FrameworkFactKind::OrmEntity => Some(FactCategory::Entity),
        FrameworkFactKind::OrmAccess => Some(FactCategory::DbAccess),
        FrameworkFactKind::ConfigRead => Some(FactCategory::EnvRead),
        FrameworkFactKind::TestSuite => Some(FactCategory::TestSuite),
        FrameworkFactKind::TestCase => Some(FactCategory::TestCase),
        FrameworkFactKind::HttpGlobalConfig => Some(FactCategory::GlobalPrefix),
        FrameworkFactKind::ModuleDeclaration
        | FrameworkFactKind::DiInjection
        | FrameworkFactKind::GlobalProvider
        | FrameworkFactKind::MetadataDecoratorDefinition
        | FrameworkFactKind::OrmColumn
        | FrameworkFactKind::OrmRelation
        | FrameworkFactKind::QueueRegistration
        | FrameworkFactKind::TestMock
        | FrameworkFactKind::Custom(_) => None,
    }
}

/// Why a fact was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FactIssueCode {
    /// A required attribute is absent.
    MissingAttribute,
    /// A present attribute has the wrong shape (not a string, empty, an unknown `op`).
    InvalidAttribute,
    /// `db_access` named an entity that no `entity` fact in this snapshot declares.
    UnresolvedEntity,
    /// The `symbol` attribute names a declaration this file does not contain.
    UnknownSymbol,
}

impl FactIssueCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingAttribute => "missing_attribute",
            Self::InvalidAttribute => "invalid_attribute",
            Self::UnresolvedEntity => "unresolved_entity",
            Self::UnknownSymbol => "unknown_symbol",
        }
    }
}

impl fmt::Display for FactIssueCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A diagnostic about one fact. Recorded, never fatal (CG-006).
#[derive(Debug, Clone, PartialEq)]
pub struct FactIssue {
    pub code: FactIssueCode,
    pub category: FactCategory,
    pub file: RepoPath,
    pub range: SourceRange,
    pub detail: String,
}

impl fmt::Display for FactIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}:{}: {} ({})",
            self.code, self.file, self.range.start.line, self.detail, self.category
        )
    }
}

/// Reads one required string attribute, or records the issue that stopped it.
pub fn require_str(
    fact: &IrFrameworkFact,
    category: FactCategory,
    file: &RepoPath,
    name: &str,
    issues: &mut Vec<FactIssue>,
) -> Option<String> {
    match fact.attrs.get(name) {
        Some(AttrValue::Str(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(_) => {
            issues.push(FactIssue {
                code: FactIssueCode::InvalidAttribute,
                category,
                file: file.clone(),
                range: fact.range,
                detail: format!("attribute {name} is not a non-empty string"),
            });
            None
        }
        None => {
            issues.push(FactIssue {
                code: FactIssueCode::MissingAttribute,
                category,
                file: file.clone(),
                range: fact.range,
                detail: format!("attribute {name} is required for category {category}"),
            });
            None
        }
    }
}

/// Reads an optional string attribute; an absent one is fine, a malformed one is not.
pub fn optional_str(
    fact: &IrFrameworkFact,
    category: FactCategory,
    file: &RepoPath,
    name: &str,
    issues: &mut Vec<FactIssue>,
) -> Option<String> {
    match fact.attrs.get(name) {
        None => None,
        Some(AttrValue::Str(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(_) => {
            issues.push(FactIssue {
                code: FactIssueCode::InvalidAttribute,
                category,
                file: file.clone(),
                range: fact.range,
                detail: format!("attribute {name} is not a non-empty string"),
            });
            None
        }
    }
}

/// Reads a list-of-strings attribute (`targets`), accepting either a `List` or a single string.
pub fn str_list(
    fact: &IrFrameworkFact,
    category: FactCategory,
    file: &RepoPath,
    name: &str,
    issues: &mut Vec<FactIssue>,
) -> Vec<String> {
    match fact.attrs.get(name) {
        None => Vec::new(),
        Some(AttrValue::Str(value)) => vec![value.clone()],
        Some(AttrValue::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                AttrValue::Str(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        Some(_) => {
            issues.push(FactIssue {
                code: FactIssueCode::InvalidAttribute,
                category,
                file: file.clone(),
                range: fact.range,
                detail: format!("attribute {name} is not a string or a list of strings"),
            });
            Vec::new()
        }
    }
}

/// Reads a boolean attribute, defaulting to `false` when absent.
pub fn flag(fact: &IrFrameworkFact, name: &str) -> bool {
    matches!(fact.attrs.get(name), Some(AttrValue::Bool(true)))
}

/// Normalizes a source range written as `[start_line, start_col, end_line, end_col]`, which is
/// how the adapters serialize it into an attribute.
#[must_use]
pub fn range_attr(value: Option<&AttrValue>) -> Option<SourceRange> {
    let AttrValue::List(items) = value? else {
        return None;
    };
    let mut numbers = [0u32; 4];
    for (slot, item) in numbers.iter_mut().zip(items) {
        let AttrValue::Int(raw) = item else {
            return None;
        };
        *slot = u32::try_from(*raw).ok()?;
    }
    review_core::location::SourceRange::new(
        review_core::location::Position::new(numbers[0], numbers[1]).ok()?,
        review_core::location::Position::new(numbers[2], numbers[3]).ok()?,
    )
    .ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use analysis_ir::symbol::LocalId;
    use review_core::location::{Position, SourceRange};

    fn range() -> SourceRange {
        SourceRange::new(Position::new(1, 0).unwrap(), Position::new(2, 0).unwrap()).unwrap()
    }

    fn make_fact(kind: FrameworkFactKind, attrs: Vec<(&str, AttrValue)>) -> IrFrameworkFact {
        IrFrameworkFact {
            adapter: "test".to_owned(),
            kind,
            symbol: Some(LocalId(1)),
            attrs: attrs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            range: range(),
            confidence: 0.9,
        }
    }

    #[test]
    fn categories_cover_the_documented_contract() {
        assert_eq!(
            category_of(&FrameworkFactKind::Controller),
            Some(FactCategory::Controller)
        );
        assert_eq!(
            category_of(&FrameworkFactKind::HttpRoute),
            Some(FactCategory::Route)
        );
        assert_eq!(
            category_of(&FrameworkFactKind::Middleware),
            Some(FactCategory::Guard)
        );
        assert_eq!(
            category_of(&FrameworkFactKind::QueueJobHandler),
            Some(FactCategory::QueueConsumer)
        );
        assert_eq!(
            category_of(&FrameworkFactKind::OrmAccess),
            Some(FactCategory::DbAccess)
        );
        assert_eq!(
            category_of(&FrameworkFactKind::ConfigRead),
            Some(FactCategory::EnvRead)
        );
        assert_eq!(category_of(&FrameworkFactKind::DiInjection), None);
        assert_eq!(category_of(&FrameworkFactKind::OrmColumn), None);
        assert_eq!(category_of(&FrameworkFactKind::Custom("x".into())), None);
        assert_eq!(FactCategory::ALL.len(), FactCategory::REQUIRED.len());
    }

    #[test]
    fn required_attribute_table_names_every_category() {
        for category in FactCategory::ALL {
            let listed: Vec<&str> = FactCategory::REQUIRED
                .iter()
                .find(|(name, _)| *name == category.as_str())
                .map(|(_, attrs)| attrs.split(',').collect())
                .unwrap_or_default();
            assert!(!listed.is_empty(), "{category} has no contract row");
            for attr in listed {
                assert!(!attr.is_empty(), "{category} lists an empty attribute");
            }
        }
    }

    #[test]
    fn missing_and_malformed_attributes_become_issues() {
        let path = RepoPath::new("src/a.ts").unwrap();
        let mut issues = Vec::new();
        let fact = make_fact(FrameworkFactKind::Controller, vec![]);
        assert_eq!(
            require_str(
                &fact,
                FactCategory::Controller,
                &path,
                "symbol",
                &mut issues
            ),
            None
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, FactIssueCode::MissingAttribute);

        let mut issues = Vec::new();
        let fact = make_fact(
            FrameworkFactKind::Controller,
            vec![("symbol", AttrValue::Int(3))],
        );
        assert_eq!(
            require_str(
                &fact,
                FactCategory::Controller,
                &path,
                "symbol",
                &mut issues
            ),
            None
        );
        assert_eq!(issues[0].code, FactIssueCode::InvalidAttribute);

        let mut issues = Vec::new();
        let fact = make_fact(
            FrameworkFactKind::Controller,
            vec![("symbol", AttrValue::Str("  ".to_owned()))],
        );
        assert_eq!(
            require_str(
                &fact,
                FactCategory::Controller,
                &path,
                "symbol",
                &mut issues
            ),
            None
        );
        assert_eq!(issues[0].code, FactIssueCode::InvalidAttribute);

        let mut issues = Vec::new();
        let fact = make_fact(
            FrameworkFactKind::Controller,
            vec![("symbol", AttrValue::Str("A".to_owned()))],
        );
        assert_eq!(
            require_str(
                &fact,
                FactCategory::Controller,
                &path,
                "symbol",
                &mut issues
            ),
            Some("A".to_owned())
        );
        assert!(issues.is_empty());
        assert_eq!(
            optional_str(
                &fact,
                FactCategory::Controller,
                &path,
                "absent",
                &mut issues
            ),
            None
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn list_attributes_accept_a_bare_string() {
        let path = RepoPath::new("src/a.ts").unwrap();
        let mut issues = Vec::new();
        let fact = make_fact(
            FrameworkFactKind::Middleware,
            vec![("targets", AttrValue::Str("A".to_owned()))],
        );
        assert_eq!(
            str_list(&fact, FactCategory::Guard, &path, "targets", &mut issues),
            vec!["A".to_owned()]
        );
        let fact = make_fact(
            FrameworkFactKind::Middleware,
            vec![(
                "targets",
                AttrValue::List(vec![
                    AttrValue::Str("A".to_owned()),
                    AttrValue::Int(1),
                    AttrValue::Str("B".to_owned()),
                ]),
            )],
        );
        assert_eq!(
            str_list(&fact, FactCategory::Guard, &path, "targets", &mut issues),
            vec!["A".to_owned(), "B".to_owned()]
        );
        let fact = make_fact(
            FrameworkFactKind::Middleware,
            vec![("targets", AttrValue::Bool(true))],
        );
        assert!(str_list(&fact, FactCategory::Guard, &path, "targets", &mut issues).is_empty());
        assert_eq!(issues[0].code, FactIssueCode::InvalidAttribute);
    }

    #[test]
    fn ranges_parse_from_a_four_number_list() {
        let good = AttrValue::List(vec![
            AttrValue::Int(3),
            AttrValue::Int(2),
            AttrValue::Int(4),
            AttrValue::Int(1),
        ]);
        let parsed = range_attr(Some(&good)).unwrap();
        assert_eq!(parsed.start.line, 3);
        assert_eq!(parsed.start.column, 2);
        assert_eq!(parsed.end.line, 4);
        assert_eq!(parsed.end.column, 1);

        assert!(range_attr(None).is_none());
        assert!(range_attr(Some(&AttrValue::Str("x".to_owned()))).is_none());
        assert!(range_attr(Some(&AttrValue::List(vec![AttrValue::Int(1)]))).is_none());
        assert!(range_attr(Some(&AttrValue::List(vec![
            AttrValue::Int(-1),
            AttrValue::Int(0),
            AttrValue::Int(1),
            AttrValue::Int(0)
        ])))
        .is_none());
        assert!(range_attr(Some(&AttrValue::List(vec![
            AttrValue::Int(4),
            AttrValue::Int(0),
            AttrValue::Int(1),
            AttrValue::Int(0)
        ])))
        .is_none());
        // A zero-length range is legal here: a single-line test body is exactly that.
        assert!(range_attr(Some(&AttrValue::List(vec![
            AttrValue::Int(1),
            AttrValue::Int(0),
            AttrValue::Int(1),
            AttrValue::Int(0)
        ])))
        .is_some());
    }

    #[test]
    fn flags_default_to_false() {
        let fact = make_fact(FrameworkFactKind::Middleware, vec![]);
        assert!(!flag(&fact, "global"));
        let fact = make_fact(
            FrameworkFactKind::Middleware,
            vec![("global", AttrValue::Bool(true))],
        );
        assert!(flag(&fact, "global"));
    }
}
