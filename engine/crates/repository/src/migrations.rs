//! Migration directories and schema files (INIT-009).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::walk::FileInventory;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MigrationTool {
    TypeOrm,
    Prisma,
    Knex,
    Flyway,
    Liquibase,
    RawSql,
    Alembic,
    GoMigrate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MigrationNaming {
    TimestampPrefix,
    VersionPrefix,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MigrationDirFact {
    pub dir: RepoPath,
    pub tool_hint: Option<MigrationTool>,
    pub files: u64,
    pub naming: MigrationNaming,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct MigrationFacts {
    pub dirs: Vec<MigrationDirFact>,
    pub schema_files: Vec<RepoPath>,
}

const NAMED_DIRS: &[&str] = &[
    "migrations",
    "migration",
    "db/migrate",
    "prisma/migrations",
    "schema/migrations",
    "alembic/versions",
    "db/migration",
];

fn regexes() -> &'static Option<(Regex, Regex, Regex)> {
    static RE: OnceLock<Option<(Regex, Regex, Regex)>> = OnceLock::new();
    RE.get_or_init(|| {
        Some((
            Regex::new(r"^\d{8,14}[_-]").ok()?,
            Regex::new(r"^(V\d+(\.\d+)*__|\d{1,6}_)").ok()?,
            Regex::new(r"^\d{3,}_.*\.(up|down)\.sql$").ok()?,
        ))
    })
}

fn is_named_dir(dir: &str) -> bool {
    NAMED_DIRS
        .iter()
        .any(|n| dir == *n || dir.ends_with(&format!("/{n}")))
}

/// Detects migration directories and schema files.
pub fn detect_migrations(inv: &FileInventory) -> MigrationFacts {
    let Some((timestamp, version, go_migrate)) = regexes() else {
        return MigrationFacts::default();
    };
    // dir -> immediate child names (files and sub-directories) and recursive file count
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    let dirs: Vec<&str> = inv.dirs.iter().map(|d| d.as_str()).collect();
    for dir in &dirs {
        children.entry(dir).or_default();
    }
    for dir in &dirs {
        if let Some((parent, name)) = dir.rsplit_once('/') {
            children.entry(parent).or_default().push(name);
        }
    }
    for entry in &inv.entries {
        let path = entry.path.as_str();
        if let Some((parent, name)) = path.rsplit_once('/') {
            children.entry(parent).or_default().push(name);
        }
        let mut cursor = path;
        while let Some((parent, _)) = cursor.rsplit_once('/') {
            *counts.entry(parent).or_insert(0) += 1;
            cursor = parent;
        }
    }

    let mut result = Vec::new();
    for (dir, names) in &children {
        let sql_timestamped = names
            .iter()
            .filter(|n| n.ends_with(".sql") && timestamp.is_match(n))
            .count();
        if !(is_named_dir(dir) || sql_timestamped >= 3) {
            continue;
        }
        let ts_count = names.iter().filter(|n| timestamp.is_match(n)).count();
        let ver_count = names.iter().filter(|n| version.is_match(n)).count();
        let naming = if ts_count * 2 > names.len() && ts_count > 0 {
            MigrationNaming::TimestampPrefix
        } else if ver_count * 2 > names.len() && ver_count > 0 {
            MigrationNaming::VersionPrefix
        } else {
            MigrationNaming::Other
        };
        let any_ext = |ext: &str| names.iter().any(|n| n.ends_with(ext));
        let tool_hint = if dir.ends_with("prisma/migrations") {
            Some(MigrationTool::Prisma)
        } else if dir.ends_with("alembic/versions") {
            Some(MigrationTool::Alembic)
        } else if dir.ends_with("db/migration") && names.iter().any(|n| n.starts_with('V')) {
            Some(MigrationTool::Flyway)
        } else if names.iter().any(|n| go_migrate.is_match(n)) {
            Some(MigrationTool::GoMigrate)
        } else if any_ext(".xml") && names.iter().any(|n| n.contains("changelog")) {
            Some(MigrationTool::Liquibase)
        } else if (any_ext(".ts") || any_ext(".js"))
            && names.iter().any(|n| {
                n.split('-')
                    .next()
                    .is_some_and(|p| p.len() >= 10 && p.chars().all(|c| c.is_ascii_digit()))
            })
        {
            Some(MigrationTool::TypeOrm)
        } else if (any_ext(".ts") || any_ext(".js")) && naming == MigrationNaming::TimestampPrefix {
            Some(MigrationTool::Knex)
        } else if any_ext(".sql") {
            Some(MigrationTool::RawSql)
        } else {
            None
        };
        if let Ok(path) = RepoPath::new(*dir) {
            result.push(MigrationDirFact {
                dir: path,
                tool_hint,
                files: counts.get(dir).copied().unwrap_or(0),
                naming,
            });
        }
    }
    // Nested migration directories under a named one (prisma/migrations/<ts>_name) are noise.
    let named: Vec<String> = result.iter().map(|m| m.dir.as_str().to_owned()).collect();
    result.retain(|m| {
        !named.iter().any(|other| {
            other != m.dir.as_str() && m.dir.as_str().starts_with(&format!("{other}/"))
        })
    });

    let mut schema_files: Vec<RepoPath> = inv
        .entries
        .iter()
        .filter(|e| {
            let path = e.path.as_str();
            let name = path.rsplit('/').next().unwrap_or(path);
            name == "schema.prisma"
                || (name.ends_with(".sql") && name.to_ascii_lowercase().contains("schema"))
                || (name.ends_with(".sql")
                    && path.rsplit_once('/').is_some_and(|(parent, _)| {
                        parent == "schema" || parent.ends_with("/schema")
                    }))
        })
        .map(|e| e.path.clone())
        .collect();
    schema_files.sort();
    MigrationFacts {
        dirs: result,
        schema_files,
    }
}
