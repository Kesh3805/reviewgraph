//! Typed Qdrant filter AST (SEM-003).
//!
//! Keys are `&'static str` constants from [`fields`]; there is no way to filter on a free-form
//! key, which keeps the tenant-filter audit (SEM-005, SEC-002) a simple key comparison.

use serde_json::{json, Map, Value};
use uuid::Uuid;

/// Payload field names. The only keys a filter can use.
pub mod fields {
    pub const ORGANIZATION_ID: &str = "organization_id";
    pub const REPOSITORY_ID: &str = "repository_id";
    pub const KIND: &str = "kind";
    pub const LANGUAGE: &str = "language";
    pub const MODULE: &str = "module";
    pub const SYMBOL_KEY: &str = "symbol_key";
    pub const CHUNK_KEY: &str = "chunk_key";
    pub const UNIT_KEY: &str = "unit_key";
    pub const FILE_PATH: &str = "file_path";
    pub const CONTENT_HASH: &str = "content_hash";
    pub const SNAPSHOT_IDS: &str = "snapshot_ids";
    pub const EMBEDDING_VERSION: &str = "embedding_version";
    pub const START_LINE: &str = "start_line";
    pub const END_LINE: &str = "end_line";
    pub const SYMBOL_ID: &str = "symbol_id";

    /// Keys that carry the tenant boundary.
    pub const TENANT_KEYS: [&str; 2] = [ORGANIZATION_ID, REPOSITORY_ID];

    /// Every key that gets a payload index at bootstrap (SEM-004), with its schema.
    pub const INDEXED: [(&str, super::IndexSchema); 11] = [
        (ORGANIZATION_ID, super::IndexSchema::Keyword),
        (REPOSITORY_ID, super::IndexSchema::Keyword),
        (KIND, super::IndexSchema::Keyword),
        (LANGUAGE, super::IndexSchema::Keyword),
        (MODULE, super::IndexSchema::Keyword),
        (SYMBOL_KEY, super::IndexSchema::Keyword),
        (CHUNK_KEY, super::IndexSchema::Keyword),
        (FILE_PATH, super::IndexSchema::Keyword),
        (CONTENT_HASH, super::IndexSchema::Keyword),
        (SNAPSHOT_IDS, super::IndexSchema::Keyword),
        (EMBEDDING_VERSION, super::IndexSchema::Integer),
    ];
}

/// Payload index type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexSchema {
    Keyword,
    Integer,
    Uuid,
}

impl IndexSchema {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Integer => "integer",
            Self::Uuid => "uuid",
        }
    }
}

/// A value to match.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Keyword(String),
    Integer(i64),
    Bool(bool),
}

impl FieldValue {
    pub fn keyword(s: impl ToString) -> Self {
        Self::Keyword(s.to_string())
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Keyword(s) => json!(s),
            Self::Integer(i) => json!(i),
            Self::Bool(b) => json!(b),
        }
    }
}

/// One filter condition.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Match {
        key: &'static str,
        value: FieldValue,
    },
    MatchAny {
        key: &'static str,
        values: Vec<FieldValue>,
    },
    Range {
        key: &'static str,
        gte: Option<f64>,
        lte: Option<f64>,
    },
    /// Point id membership.
    HasId(Vec<Uuid>),
}

impl Cond {
    pub fn keyword(key: &'static str, value: impl ToString) -> Self {
        Self::Match {
            key,
            value: FieldValue::keyword(value),
        }
    }

    pub fn any<T: ToString>(key: &'static str, values: impl IntoIterator<Item = T>) -> Self {
        Self::MatchAny {
            key,
            values: values.into_iter().map(FieldValue::keyword).collect(),
        }
    }

    /// The payload key this condition reads (`None` for id conditions).
    pub fn key(&self) -> Option<&'static str> {
        match self {
            Self::Match { key, .. } | Self::MatchAny { key, .. } | Self::Range { key, .. } => {
                Some(key)
            }
            Self::HasId(_) => None,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Match { key, value } => json!({"key": key, "match": {"value": value.to_json()}}),
            Self::MatchAny { key, values } => json!({
                "key": key,
                "match": {"any": values.iter().map(FieldValue::to_json).collect::<Vec<_>>()},
            }),
            Self::Range { key, gte, lte } => {
                let mut range = Map::new();
                if let Some(g) = gte {
                    range.insert("gte".into(), json!(g));
                }
                if let Some(l) = lte {
                    range.insert("lte".into(), json!(l));
                }
                json!({"key": key, "range": Value::Object(range)})
            }
            Self::HasId(ids) => json!({
                "has_id": ids.iter().map(|u| u.hyphenated().to_string()).collect::<Vec<_>>()
            }),
        }
    }
}

/// `must` (AND), `should` (OR, at least one) and `must_not` (none) clauses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub must: Vec<Cond>,
    pub should: Vec<Cond>,
    pub must_not: Vec<Cond>,
}

impl Filter {
    /// Qdrant JSON. Empty clauses are omitted.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        for (name, conds) in [
            ("must", &self.must),
            ("should", &self.should),
            ("must_not", &self.must_not),
        ] {
            if !conds.is_empty() {
                out.insert(
                    name.into(),
                    Value::Array(conds.iter().map(Cond::to_json).collect()),
                );
            }
        }
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_serialization_golden() {
        let f = Filter {
            must: vec![
                Cond::keyword(fields::ORGANIZATION_ID, "org-a"),
                Cond::any(fields::REPOSITORY_ID, ["r1", "r2"]),
                Cond::Range {
                    key: fields::EMBEDDING_VERSION,
                    gte: Some(1.0),
                    lte: None,
                },
            ],
            should: vec![],
            must_not: vec![Cond::HasId(vec![Uuid::nil()])],
        };
        assert_eq!(
            f.to_json(),
            json!({
                "must": [
                    {"key": "organization_id", "match": {"value": "org-a"}},
                    {"key": "repository_id", "match": {"any": ["r1", "r2"]}},
                    {"key": "embedding_version", "range": {"gte": 1.0}}
                ],
                "must_not": [{"has_id": ["00000000-0000-0000-0000-000000000000"]}]
            })
        );
    }
}
