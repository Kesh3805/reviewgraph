//! Database-access heuristics for `DbWriteLike` / `DbReadLike` facts (TSA-006).
//!
//! A call qualifies only when its method name is in a table *and* its receiver looks like a data
//! access object: a declared type ending in `Repository`, `EntityManager`, `DataSource`,
//! `QueryRunner`, `Model`, `Prisma` or `Knex`; a receiver name containing `repo`, `repository`,
//! `manager`, `db`, `dataSource`, `queryRunner`, `prisma` or `knex` (or exactly `em`); or a chain
//! that goes through `createQueryBuilder`. `fileStore.save(...)` therefore stays a plain call.

use crate::visit::fact_keys::Callee;

/// Method names that write.
pub const WRITE_METHODS: &[&str] = &[
    "save",
    "insert",
    "update",
    "upsert",
    "delete",
    "remove",
    "softDelete",
    "softRemove",
    "restore",
    "increment",
    "decrement",
    "destroy",
    "bulkCreate",
];

/// Method names that read.
pub const READ_METHODS: &[&str] = &[
    "find",
    "findOne",
    "findOneBy",
    "findBy",
    "findAndCount",
    "count",
    "exists",
    "getMany",
    "getOne",
    "getRawMany",
];

/// Declared-type heads that mark a typed data-access receiver.
const TYPE_SUFFIXES: &[&str] = &[
    "repository",
    "entitymanager",
    "datasource",
    "queryrunner",
    "model",
    "prisma",
    "knex",
];

/// Receiver-name fragments that mark a data-access receiver.
const NAME_FRAGMENTS: &[&str] = &[
    "repo",
    "repository",
    "manager",
    "db",
    "datasource",
    "queryrunner",
    "prisma",
    "knex",
];

/// Read or write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbAccess {
    Write,
    Read,
}

/// How the receiver qualified, which sets the confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// The receiver's declared type matched (0.9).
    Typed,
    /// Only the receiver's name or the query-builder chain matched (0.75).
    NameOnly,
    /// A raw SQL string classified by its first keyword (0.6).
    RawSql,
}

impl Evidence {
    pub const fn confidence(self) -> f64 {
        match self {
            Self::Typed => 0.9,
            Self::NameOnly => 0.75,
            Self::RawSql => 0.6,
        }
    }
}

/// A classified database call.
#[derive(Debug, Clone, PartialEq)]
pub struct DbCall {
    pub access: DbAccess,
    pub method: String,
    pub entity: Option<String>,
    pub evidence: Evidence,
}

/// The head of a declared type without generics: `Repository<User>` gives `Repository`.
pub fn type_head(declared: &str) -> &str {
    let trimmed = declared.trim();
    let end = trimmed.find('<').unwrap_or(trimmed.len());
    let head = trimmed[..end].trim();
    head.rsplit('.').next().unwrap_or(head)
}

/// The first generic argument of a declared type: `Repository<User>` gives `User`.
pub fn generic_arg(declared: &str) -> Option<String> {
    let start = declared.find('<')? + 1;
    let end = declared.rfind('>')?;
    if end <= start {
        return None;
    }
    let inner = declared[start..end].split(',').next()?.trim();
    let inner = inner.split('<').next().unwrap_or(inner).trim();
    (!inner.is_empty()).then(|| inner.to_owned())
}

/// Whether a declared type marks a data-access receiver.
pub fn is_db_type(declared: &str) -> bool {
    let head = type_head(declared).to_ascii_lowercase();
    TYPE_SUFFIXES.iter().any(|suffix| head.ends_with(suffix))
}

/// Whether a receiver name marks a data-access receiver.
pub fn is_db_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "em"
        || NAME_FRAGMENTS
            .iter()
            .any(|fragment| lower.contains(fragment))
}

/// The SQL verb class of a raw query string, by its first keyword only.
pub fn sql_access(sql: &str) -> Option<DbAccess> {
    let first = sql
        .trim_start()
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()?
        .to_ascii_uppercase();
    match first.as_str() {
        "INSERT" | "UPDATE" | "DELETE" | "ALTER" | "DROP" | "TRUNCATE" => Some(DbAccess::Write),
        "SELECT" | "WITH" => Some(DbAccess::Read),
        _ => None,
    }
}

/// Classifies a call. `receiver_type` is the declared type of the receiver when known,
/// `first_string_arg` the first argument when it is a string literal (for raw `query`), and
/// `first_ident_arg` the first argument when it is an identifier (the entity, if no generic).
pub fn classify(
    callee: &Callee,
    receiver_type: Option<&str>,
    first_string_arg: Option<&str>,
    first_ident_arg: Option<&str>,
) -> Option<DbCall> {
    let name = callee.name.as_str();
    let typed = receiver_type.is_some_and(is_db_type);
    let named = callee.receiver_name().is_some_and(is_db_name);
    let builder = callee.chain_has_call("createQueryBuilder");
    if !(typed || named || builder) {
        return None;
    }
    // Inside a query-builder chain only the terminal call touches the database: `.update(User)`
    // and `.where(...)` are builder steps, `.execute()` / `.getMany()` run the query.
    if builder && !matches!(name, "execute" | "getMany" | "getOne" | "getRawMany") {
        return None;
    }
    let evidence = if typed {
        Evidence::Typed
    } else {
        Evidence::NameOnly
    };
    let entity = receiver_type
        .and_then(generic_arg)
        .or_else(|| first_ident_arg.map(str::to_owned));

    let access = match name {
        "query" => {
            let access = sql_access(first_string_arg?)?;
            return Some(DbCall {
                access,
                method: name.to_owned(),
                entity: None,
                evidence: Evidence::RawSql,
            });
        }
        "execute" => {
            let writes = ["insert", "update", "delete"]
                .iter()
                .any(|step| callee.chain_has_call(step));
            if !writes {
                return None;
            }
            DbAccess::Write
        }
        "create" => {
            // `repository.create(...)` only builds an entity in memory; a Model-like receiver
            // (`UserModel.create`) persists it.
            let model_type =
                receiver_type.is_some_and(|t| type_head(t).to_ascii_lowercase().ends_with("model"));
            let model_name = callee
                .receiver_name()
                .is_some_and(|n| n.to_ascii_lowercase().ends_with("model"));
            if !(model_type || model_name) {
                return None;
            }
            DbAccess::Write
        }
        _ if WRITE_METHODS.contains(&name) => DbAccess::Write,
        _ if READ_METHODS.contains(&name) => DbAccess::Read,
        _ => return None,
    };
    Some(DbCall {
        access,
        method: name.to_owned(),
        entity,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callee(receiver: &[&str], name: &str) -> Callee {
        Callee {
            receiver: receiver.iter().map(|s| (*s).to_owned()).collect(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn typed_repository_save_is_a_write() {
        let call = classify(
            &callee(&["this", "users"], "save"),
            Some("Repository<User>"),
            None,
            Some("user"),
        )
        .unwrap_or_else(|| unreachable!());
        assert_eq!(call.access, DbAccess::Write);
        assert_eq!(call.entity.as_deref(), Some("User"));
        assert_eq!(call.evidence, Evidence::Typed);
    }

    #[test]
    fn plain_store_save_is_not_db() {
        assert!(classify(&callee(&["this", "fileStore"], "save"), None, None, None).is_none());
        assert!(classify(&callee(&["items"], "find"), None, None, None).is_none());
    }

    #[test]
    fn raw_query_by_verb() {
        let write = classify(
            &callee(&["this", "dataSource"], "query"),
            None,
            Some("  update users set x = 1"),
            None,
        );
        assert_eq!(write.map(|c| c.access), Some(DbAccess::Write));
        let read = classify(
            &callee(&["this", "dataSource"], "query"),
            None,
            Some("SELECT 1"),
            None,
        );
        assert_eq!(read.map(|c| c.evidence), Some(Evidence::RawSql));
    }

    #[test]
    fn type_helpers() {
        assert_eq!(type_head("typeorm.Repository<User>"), "Repository");
        assert_eq!(generic_arg("Repository<User>").as_deref(), Some("User"));
        assert_eq!(generic_arg("Repository"), None);
    }
}
